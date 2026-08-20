//! Replay appliers for orchestration events.
//!
//! Handlers and replay appliers are independent testable concerns: handlers
//! validate and transition live state, while appliers reconstruct state from
//! the immutable event log. This module owns the orchestration replay side
//! (`SearchCompleted`, `HarnessRunCompleted`, `ApprovalRecorded`,
//! `TickObserved`, `SearchExecuted`, `SearchKnowledgeCompleted`) so
//! `orchestration.rs` keeps one concept.

use crate::types::*;

impl KernelState {
    pub(crate) fn apply_search_completed(
        &mut self,
        artifact_id: ArtifactId,
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        // SearchCompleted must never touch pending parser metadata.
        Ok(())
    }

    pub(crate) fn apply_harness_run_completed(
        &mut self,
        task_id: Option<TaskId>,
    ) -> Result<(), DomainError> {
        match task_id {
            Some(id) if !self.tasks.contains_key(&id) => Err(DomainError::MissingTask { id }),
            _ => Ok(()),
        }
    }

    pub(crate) fn apply_approval_recorded(
        &mut self,
        approval_id: ApprovalId,
        outcome: ApprovalOutcome,
    ) -> Result<(), DomainError> {
        match outcome {
            ApprovalOutcome::Acknowledged { .. } => {
                self.resolved_approvals.insert(approval_id);
            }
            ApprovalOutcome::TaskTransition {
                task_id,
                from_status,
                to_status,
                ..
            } => {
                let Some(task) = self.tasks.get_mut(&task_id) else {
                    return Err(DomainError::MissingTask { id: task_id });
                };
                if task.status != from_status {
                    return Err(DomainError::InternalInvariantViolation {
                        detail: "approval replay: task status does not match from_status",
                    });
                }
                if from_status != to_status {
                    let valid = (from_status == TaskStatus::Draft
                        && to_status == TaskStatus::Active)
                        || from_status.can_transition_to(to_status);
                    if !valid {
                        return Err(DomainError::InternalInvariantViolation {
                            detail: "approval replay: invalid status transition in ApprovalRecorded",
                        });
                    }
                    task.status = to_status;
                }
                self.resolved_approvals.insert(approval_id);
            }
        }
        Ok(())
    }

    pub(crate) fn apply_tick_observed(&mut self, at: LogicalTick) {
        self.current_tick = Some(at);
    }

    pub(crate) fn apply_search_executed(&mut self, query: &str) -> Result<(), DomainError> {
        if query.trim().is_empty() {
            return Err(DomainError::EmptyIntent);
        }
        // SearchExecuted is a pure audit event — no state mutation on replay.
        Ok(())
    }

    pub(crate) fn apply_search_knowledge_completed(&mut self) -> Result<(), DomainError> {
        Ok(())
    }
}
