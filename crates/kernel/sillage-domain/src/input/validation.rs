use crate::types::*;
use std::sync::Arc;

impl KernelState {
    // ── Handlers ─────────────────────────────────────────────────

    pub(super) fn handle_record_validation_report(
        &mut self,
        input: RecordValidationReportInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        self.apply_validation_report_created(
            input.report_id,
            input.task_id,
            input.passed,
            &input.warnings,
        )?;
        Ok(self.emit_event(DomainEvent::ValidationReportCreated {
            report_id: input.report_id,
            task_id: input.task_id,
            passed: input.passed,
            warnings: input.warnings,
        }))
    }

    pub(super) fn handle_validation_completed(
        &mut self,
        input: ValidationCompleted,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let status = if input.valid {
            ClaimStatus::Verified
        } else {
            ClaimStatus::Disputed
        };

        let claim = Arc::make_mut(&mut self.claims)
            .get_mut(&input.claim_id)
            .ok_or(DomainError::MissingClaim { id: input.claim_id })?;
        claim.status = status.clone();

        Ok(self.emit_event(DomainEvent::ClaimValidationUpdated {
            claim_id: input.claim_id,
            status,
        }))
    }

    // ── Replay apply ─────────────────────────────────────────────

    pub(crate) fn apply_validation_report_created(
        &mut self,
        report_id: ValidationReportId,
        task_id: Option<TaskId>,
        passed: bool,
        warnings: &[String],
    ) -> Result<(), DomainError> {
        if self.validation_reports.contains_key(&report_id) {
            return Err(DomainError::DuplicateValidationReport { id: report_id });
        }
        if let Some(tid) = task_id
            && !self.tasks.contains_key(&tid)
        {
            return Err(DomainError::MissingTask { id: tid });
        }
        Arc::make_mut(&mut self.validation_reports).insert(
            report_id,
            ValidationReportRecord {
                task_id,
                passed,
                warnings: warnings.to_vec(),
            },
        );
        Ok(())
    }
}
