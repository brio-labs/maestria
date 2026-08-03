use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use crate::harness::truncate_output;
use maestria_domain::{
    CreateMemoryCandidateInput, DomainInput, ModelAgentHarnessResult, ModelAgentMemoryDecision,
    ModelAgentMemoryResult, ModelAgentProposalRequest, ModelAgentProposalResult,
    ModelAgentSearchResult, ModelAgentValidationResult, QueryHarnessProposalRequest,
    QueryHarnessRequest, SearchKnowledgeCompleted,
};
use maestria_governance::{
    MemoryPromotionDecision, MemoryPromotionRequest, ValidationDecision, ValidationRequest,
};
use maestria_ports::{EffectJournalIntent, HarnessRequest};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn validate_proposal_search_generation(
    expected_generation: maestria_domain::IndexGenerationId,
    actual_generation: maestria_domain::IndexGenerationId,
) -> Result<(), EffectFailure> {
    if expected_generation != actual_generation {
        return Err(EffectFailure::Failed(format!(
            "proposal search generation mismatch: expected {}, got {}",
            expected_generation.value(),
            actual_generation.value()
        )));
    }
    Ok(())
}

pub(crate) fn model_agent_denial_result(
    proposal: &ModelAgentProposalRequest,
    reason: String,
) -> ModelAgentProposalResult {
    ModelAgentProposalResult::Failed {
        run_id: proposal.run_id,
        correlation_id: proposal.correlation_id,
        error: reason,
    }
}

impl EffectExecutionContext {
    pub(crate) async fn handle_query_harness_proposal(
        &self,
        request: QueryHarnessProposalRequest,
    ) -> Result<(), EffectFailure> {
        let proposal = request.proposal;
        match self.execute_model_agent_proposal(&proposal).await {
            Ok(result) => self.persist_model_agent_result(result).await,
            Err(error) => {
                let result = ModelAgentProposalResult::Failed {
                    run_id: proposal.run_id,
                    correlation_id: proposal.correlation_id,
                    error: error.to_string(),
                };
                self.persist_model_agent_result(result).await
            }
        }
    }

    pub(crate) async fn record_model_agent_denial(
        &self,
        proposal: &ModelAgentProposalRequest,
        reason: String,
    ) -> Result<(), EffectFailure> {
        self.persist_model_agent_result(model_agent_denial_result(proposal, reason))
            .await
    }

    async fn execute_model_agent_proposal(
        &self,
        proposal: &ModelAgentProposalRequest,
    ) -> Result<ModelAgentProposalResult, EffectFailure> {
        let (search, harness) = match &proposal.execution {
            maestria_domain::ModelAgentProposalExecution::Fresh
            | maestria_domain::ModelAgentProposalExecution::ApprovalContinuation { .. } => {
                let search = self.execute_proposal_search(proposal).await?;
                let harness = self.execute_proposal_harness(proposal).await?;
                (search, harness)
            }
            maestria_domain::ModelAgentProposalExecution::JournalRecovery {
                journal_generation,
            } => {
                let harness = self
                    .execute_recovered_harness(proposal, *journal_generation)
                    .await?;
                (None, harness)
            }
        };
        let validation = self.evaluate_proposal_validation(proposal).await;
        let memory_candidate = self
            .create_proposal_memory_candidate(proposal, harness.is_some())
            .await?;
        Ok(ModelAgentProposalResult::Succeeded {
            run_id: proposal.run_id,
            correlation_id: proposal.correlation_id,
            search,
            harness,
            validation,
            memory_candidate,
        })
    }

    async fn execute_proposal_search(
        &self,
        proposal: &ModelAgentProposalRequest,
    ) -> Result<Option<ModelAgentSearchResult>, EffectFailure> {
        if proposal.query.trim().is_empty() {
            return Ok(None);
        }
        let executor = self.adapters.search_executor.as_ref().ok_or_else(|| {
            EffectFailure::Failed("model-agent proposal has no search executor".to_string())
        })?;
        let (plan, outcome) = executor
            .plan_and_search(proposal.query.clone(), proposal.limit)
            .await
            .map_err(|error| EffectFailure::Failed(format!("proposal search failed: {error}")))?;
        validate_proposal_search_generation(proposal.expected_generation, plan.index_generation())?;
        let result = ModelAgentSearchResult {
            trace_id: outcome.trace,
            evidence_count: outcome.evidence.len(),
        };
        self.input_tx
            .send(DomainInput::SearchKnowledgeCompleted(
                SearchKnowledgeCompleted {
                    task_id: proposal.task_id,
                    plan: Box::new(plan),
                    outcome,
                },
            ))
            .await
            .map_err(|error| {
                EffectFailure::Degraded(format!("deliver proposal search result: {error}"))
            })?;
        Ok(Some(result))
    }

    fn prepare_fresh_proposal_journal(
        &self,
        proposal: &ModelAgentProposalRequest,
    ) -> Result<u64, EffectFailure> {
        if !matches!(
            &proposal.execution,
            maestria_domain::ModelAgentProposalExecution::Fresh
        ) {
            return Err(EffectFailure::Failed(
                "only a fresh proposal can create a harness journal intent".to_string(),
            ));
        }
        self.record_harness_intent_and_start(
            EffectJournalIntent {
                run_id: proposal.run_id,
                task_id: proposal.task_id,
                capability: proposal.capability.clone(),
                command: proposal.command.clone(),
                scope_id: self.scope_id,
                requested_generation: None,
            },
            "record proposal harness intent",
            "record proposal harness start",
        )
    }

    async fn execute_proposal_harness(
        &self,
        proposal: &ModelAgentProposalRequest,
    ) -> Result<Option<ModelAgentHarnessResult>, EffectFailure> {
        if proposal.command.trim().is_empty() {
            return Ok(None);
        }
        let ordinary = QueryHarnessRequest {
            run_id: proposal.run_id,
            task_id: proposal.task_id,
            execution: match &proposal.execution {
                maestria_domain::ModelAgentProposalExecution::Fresh => {
                    maestria_domain::HarnessExecution::Fresh
                }
                maestria_domain::ModelAgentProposalExecution::JournalRecovery {
                    journal_generation,
                } => maestria_domain::HarnessExecution::JournalRecovery {
                    generation: journal_generation.value(),
                },
                maestria_domain::ModelAgentProposalExecution::ApprovalContinuation {
                    approval_id,
                    journal_generation,
                } => maestria_domain::HarnessExecution::ApprovalContinuation {
                    approval_id: *approval_id,
                    generation: journal_generation.value(),
                },
            },
            capability: proposal.capability.clone(),
            scope_id: self.scope_id,
            command: proposal.command.clone(),
        };
        let (class, default_working_directory) = self.gate_harness_request(&ordinary)?;
        let scope_guard = maestria_governance::ScopeGuard::new(self.scope.clone());
        let working_directory = if proposal.working_directory.trim().is_empty() {
            default_working_directory
        } else {
            let requested = PathBuf::from(&proposal.working_directory);
            scope_guard
                .check_read_containment(&requested)
                .map_err(|error| {
                    EffectFailure::Denied(format!(
                        "proposal working directory is outside readable scope: {error:?}"
                    ))
                })?;
            requested
        };
        let generation = match &proposal.execution {
            maestria_domain::ModelAgentProposalExecution::Fresh => {
                maestria_domain::JournalGeneration::new(
                    self.prepare_fresh_proposal_journal(proposal)?,
                )
            }
            maestria_domain::ModelAgentProposalExecution::ApprovalContinuation {
                journal_generation,
                ..
            } => *journal_generation,
            maestria_domain::ModelAgentProposalExecution::JournalRecovery { .. } => {
                return Err(EffectFailure::Failed(
                    "journal recovery cannot execute a harness provider".to_string(),
                ));
            }
        };
        let outcome = self
            .execute_and_process_harness(
                QueryHarnessRequest {
                    execution: maestria_domain::HarnessExecution::JournalRecovery {
                        generation: generation.value(),
                    },
                    ..ordinary
                },
                HarnessRequest {
                    run_id: proposal.run_id,
                    command: proposal.command.clone(),
                    working_directory,
                    duration_budget: std::time::Duration::from_secs(proposal.timeout_secs),
                    class,
                    readable_roots: scope_guard.scope().readable_roots().to_vec(),
                    blocked_paths: scope_guard.scope().blocked_paths().to_vec(),
                    blocked_patterns: scope_guard.scope().blocked_patterns().to_vec(),
                },
                generation.value(),
            )
            .await?;
        Ok(outcome.map(|outcome| ModelAgentHarnessResult {
            exit_code: outcome.exit_code,
            stdout: truncate_output(&outcome.stdout),
            stderr: truncate_output(&outcome.stderr),
            duration_ms: outcome.duration.as_millis().min(u128::from(u64::MAX)) as u64,
        }))
    }

    async fn evaluate_proposal_validation(
        &self,
        proposal: &ModelAgentProposalRequest,
    ) -> Option<ModelAgentValidationResult> {
        if !proposal.task_validation {
            return None;
        }
        let task = {
            let state = self.state.read().await;
            proposal
                .task_id
                .and_then(|task_id| state.tasks.get(&task_id).cloned())
        };
        let Some(task) = task else {
            return Some(ModelAgentValidationResult {
                passed: false,
                warnings: vec!["proposal task is unavailable for validation".to_string()],
            });
        };
        let request = ValidationRequest {
            task,
            validation_report: None,
            proposed_status: maestria_governance::ProposedCompletion::Verified,
        };
        match self.governance.validation_gate.evaluate(&request) {
            ValidationDecision::AllowCompletion => Some(ModelAgentValidationResult {
                passed: true,
                warnings: Vec::new(),
            }),
            decision => Some(ModelAgentValidationResult {
                passed: false,
                warnings: vec![format!("validation gate decision: {decision:?}")],
            }),
        }
    }

    async fn create_proposal_memory_candidate(
        &self,
        proposal: &ModelAgentProposalRequest,
        harness_completed: bool,
    ) -> Result<Option<ModelAgentMemoryResult>, EffectFailure> {
        if !proposal.memory_candidate || !harness_completed || proposal.evidence_ids.is_empty() {
            return Ok(None);
        }
        let claim_id = {
            let state = self.state.read().await;
            proposal
                .evidence_ids
                .iter()
                .filter_map(|evidence_id| state.evidences.get(evidence_id))
                .find_map(|evidence| evidence.claim_id)
        };
        let Some(claim_id) = claim_id else {
            return Ok(None);
        };
        let candidate_id = self
            .adapters
            .id_allocator
            .allocate_memory_candidate_id()
            .map_err(|error| {
                EffectFailure::Failed(format!("allocate proposal memory candidate: {error}"))
            })?;
        let security = maestria_domain::SecurityMetadata {
            scope_id: Some(self.scope_id),
            ..maestria_domain::SecurityMetadata::default()
        };
        let candidate = maestria_domain::MemoryCandidate {
            id: candidate_id,
            claim_id,
            evidence_ids: proposal
                .evidence_ids
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            confidence_milli: 800,
            security: security.clone(),
        };
        let decision = self
            .governance
            .memory_promotion_gate
            .evaluate(&MemoryPromotionRequest {
                candidate,
                user_approved: false,
            });
        let decision = match decision {
            MemoryPromotionDecision::Promote => ModelAgentMemoryDecision::Promote,
            MemoryPromotionDecision::RequireEvidence { .. } => {
                ModelAgentMemoryDecision::RequireEvidence
            }
            MemoryPromotionDecision::RequireReview { .. } => {
                ModelAgentMemoryDecision::RequireReview
            }
            MemoryPromotionDecision::Deny { .. } => ModelAgentMemoryDecision::Deny,
        };
        self.input_tx
            .send(DomainInput::CreateMemoryCandidate(
                CreateMemoryCandidateInput {
                    candidate_id,
                    claim_id,
                    evidence_ids: proposal.evidence_ids.clone(),
                    confidence_milli: 800,
                    security: Some(security),
                },
            ))
            .await
            .map_err(|error| {
                EffectFailure::Degraded(format!("deliver proposal memory candidate: {error}"))
            })?;
        Ok(Some(ModelAgentMemoryResult {
            candidate_id,
            confidence_milli: 800,
            decision,
        }))
    }

    pub(crate) async fn persist_model_agent_result(
        &self,
        result: ModelAgentProposalResult,
    ) -> Result<(), EffectFailure> {
        self.input_tx
            .send(DomainInput::ModelAgentProposalCompleted(result))
            .await
            .map_err(|error| {
                EffectFailure::Degraded(format!("persist model-agent terminal result: {error}"))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::validate_proposal_search_generation;
    use crate::effect_result::EffectFailure;

    #[test]
    fn stale_deferred_proposal_search_generation_fails_terminally() {
        let result = validate_proposal_search_generation(
            maestria_domain::IndexGenerationId::new(11),
            maestria_domain::IndexGenerationId::new(12),
        );
        assert!(
            matches!(result, Err(EffectFailure::Failed(message)) if message.contains("generation mismatch"))
        );
    }
}
