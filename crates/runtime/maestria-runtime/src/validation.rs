use crate::config::EffectExecutionContext;
use maestria_domain::{
    DomainEvent, DomainInput, RecordValidationReportInput, RunValidationRequest,
    ValidationCompleted,
};
use maestria_memory::MemoryService;
use maestria_validation::{ValidationContext, ValidationRunner};
use std::collections::BTreeMap;

impl EffectExecutionContext {
    /// Run validation pass over claims and evidence in the current kernel state.
    /// Optionally scoped to a single claim when `request.claim_id` is Some.
    /// Produces both a per-claim completion signal and a durable report input.
    pub(crate) async fn handle_run_validation(&self, request: RunValidationRequest) -> bool {
        let report = self.build_validation_report(&request).await;
        if let Some(claim_id) = request.claim_id {
            if Self::send_input(
                &self.input_tx,
                DomainInput::ValidationCompleted(ValidationCompleted {
                    claim_id,
                    valid: report.passed,
                }),
                "validation completion",
            )
            .is_err()
            {
                return false;
            }
        } else {
            tracing::debug!(task_id = ?request.task_id, "validation effect has no claim to validate");
        }
        if Self::send_input(
            &self.input_tx,
            DomainInput::RecordValidationReport(RecordValidationReportInput {
                report_id: report.id,
                task_id: request.task_id,
                passed: report.passed,
                warnings: report.warnings,
            }),
            "validation report",
        )
        .is_err()
        {
            return false;
        }
        true
    }

    async fn build_validation_report(
        &self,
        request: &RunValidationRequest,
    ) -> maestria_validation::ValidationReport {
        let state = self.state.read().await;
        let task = request
            .task_id
            .and_then(|task_id| state.tasks.get(&task_id));
        let harness_exit_code = request.task_id.and_then(|task_id| {
            state
                .event_log
                .iter()
                .rev()
                .find_map(|entry| match entry.event {
                    DomainEvent::HarnessRunCompleted {
                        task_id: Some(event_task_id),
                        exit_code,
                        ..
                    } if event_task_id == task_id => Some(exit_code),
                    _ => None,
                })
        });
        let claims = selected_claims(&state, request.claim_id);
        let evidences = state
            .evidences
            .iter()
            .map(|(id, evidence)| (*id, evidence.clone()))
            .collect();
        let memory_candidates = state
            .memory_candidates
            .iter()
            .map(|(id, candidate)| (*id, candidate.clone()))
            .collect();
        let review_queue = MemoryService::review_queue(&state.memory_candidates, &state.memories);
        if !review_queue.is_empty() {
            tracing::debug!(
                pending_candidates = review_queue.len(),
                "validation found queued memory candidates"
            );
        }
        let mut validators: Vec<Box<dyn maestria_validation::Validator>> = vec![
            Box::new(maestria_validation::CitationValidator),
            Box::new(maestria_validation::EvidenceExistenceValidator),
            Box::new(maestria_validation::MemoryValidator),
            Box::new(maestria_validation::HarnessRunValidator),
        ];
        if request.task_id.is_some() {
            validators.push(Box::new(maestria_validation::TaskStateValidator));
        }
        ValidationRunner::with_validators(validators).run(
            request.validation_report_id,
            request.task_id,
            &ValidationContext {
                task,
                claims: &claims,
                evidences: &evidences,
                memory_candidates: &memory_candidates,
                harness_exit_code,
            },
        )
    }
}

fn selected_claims(
    state: &maestria_domain::KernelState,
    claim_id: Option<maestria_domain::ClaimId>,
) -> BTreeMap<maestria_domain::ClaimId, maestria_domain::Claim> {
    let mut claims = BTreeMap::new();
    if let Some(claim_id) = claim_id {
        if let Some(claim) = state.claims.get(&claim_id) {
            claims.insert(claim_id, claim.clone());
        } else {
            tracing::warn!(claim_id = ?claim_id, "validation requested for missing claim");
        }
    } else {
        claims.extend(state.claims.iter().map(|(id, claim)| (*id, claim.clone())));
    }
    claims
}
