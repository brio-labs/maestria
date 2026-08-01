//! Entity-level event replay.
//!
//! Replay of cards, claims, evidence, tasks, relations, and validation
//! reports, dispatched by [`crate::replay::KernelState::apply_event`].
//! Kept as a sibling of `replay.rs` so each module owns one replay family.

use crate::types::*;

impl KernelState {
    pub(super) fn replay_card_events(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::CardCreated {
                card_id,
                artifact_id,
                node_id,
                source_span,
                title,
                body,
                security,
            } => self.apply_card_created(crate::input::ApplyCardCreatedArgs {
                card_id: *card_id,
                artifact_id: *artifact_id,
                node_id: *node_id,
                source_span: *source_span,
                title,
                body,
                security,
            }),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_card_events: unexpected event variant",
            }),
        }
    }

    pub(super) fn replay_claim_events(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::ClaimCreated {
                claim_id,
                artifact_id,
                text,
                evidence_ids,
                security,
            } => self.apply_claim_created(*claim_id, *artifact_id, text, evidence_ids, security),
            DomainEvent::ClaimValidationUpdated { claim_id, status } => {
                self.apply_claim_validation_updated(*claim_id, status)
            }
            DomainEvent::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => self.apply_claim_evidence_linked(*claim_id, *evidence_id),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_claim_events: unexpected event variant",
            }),
        }
    }

    pub(super) fn replay_evidence_events(
        &mut self,
        event: &DomainEvent,
    ) -> Result<(), DomainError> {
        match event {
            DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                kind,
                excerpt,
                observed_at,
                security,
            } => self.apply_evidence_recorded(crate::input::ApplyEvidenceRecordedArgs {
                evidence_id: *evidence_id,
                artifact_id: *artifact_id,
                claim_id: *claim_id,
                kind,
                excerpt,
                observed_at: *observed_at,
                security,
            }),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_evidence_events: unexpected event variant",
            }),
        }
    }

    pub(super) fn replay_task_events(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => self.apply_task_opened(*task_id, title, *priority, *artifact_id),
            DomainEvent::TaskStatusChanged { task_id, from, to } => {
                self.apply_task_status_changed(*task_id, *from, *to)
            }
            DomainEvent::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => self.apply_task_completion_recorded(*task_id, *status, *validation_report_id),
            DomainEvent::TaskEvidenceLinked {
                task_id,
                evidence_id,
            } => self.apply_task_evidence_linked(*task_id, *evidence_id),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_task_events: unexpected event variant",
            }),
        }
    }

    pub(super) fn replay_relation_events(
        &mut self,
        event: &DomainEvent,
    ) -> Result<(), DomainError> {
        match event {
            DomainEvent::RelationCreated {
                relation_id,
                source,
                kind,
                target,
                evidence_id,
                confidence_milli,
                security,
            } => self.apply_relation_created(crate::input::ApplyRelationCreatedArgs {
                relation_id: *relation_id,
                source: *source,
                kind: *kind,
                target: *target,
                evidence_id: *evidence_id,
                confidence_milli: *confidence_milli,
                security,
            }),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_relation_events: unexpected event variant",
            }),
        }
    }

    pub(super) fn replay_validation_events(
        &mut self,
        event: &DomainEvent,
    ) -> Result<(), DomainError> {
        match event {
            DomainEvent::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => self.apply_validation_report_created(*report_id, *task_id, *passed, warnings),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_validation_events: unexpected event variant",
            }),
        }
    }
}
