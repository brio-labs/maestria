//! v2 event-payload encodings for payloads reshaped in v3.
//!
//! Domain-event rows are immutable; older encodings decode through this
//! module instead of being rewritten. Payloads whose v2 encoding equals the
//! v3 encoding are parsed directly as `StoredEventPayload`; only the two
//! reshaped payloads need a versioned mirror here.

use maestria_ports::PortError;
use serde::Deserialize;

use super::event_payloads::{StoredApprovalOutcome, StoredEventPayload};
use super::evidence_payloads::StoredTaskStatus;

/// v2 encoding of `StoredEventPayload::ApprovalRecorded`: flat correlated
/// fields that v3 models as an outcome enum.
#[derive(Debug, Deserialize)]
pub(crate) struct StoredApprovalRecordedV2 {
    pub approval_id: u64,
    #[serde(default)]
    pub task_id: Option<u64>,
    pub approved: bool,
    pub from_status: Option<StoredTaskStatus>,
    pub to_status: Option<StoredTaskStatus>,
}

impl StoredApprovalRecordedV2 {
    pub(crate) fn into_v3(self) -> Result<StoredEventPayload, PortError> {
        Ok(StoredEventPayload::ApprovalRecorded {
            approval_id: self.approval_id,
            outcome: upcast_approval_outcome(
                self.task_id,
                self.approved,
                self.from_status,
                self.to_status,
            )?,
        })
    }
}

/// Upcast the v2 flat approval fields to the v3 outcome encoding.
///
/// The v2 encoding allowed the invalid mixed Some/None status pairs and
/// task-less transitions; both are rejected here so replay fails closed
/// instead of reconstructing an apparently valid domain event.
pub(crate) fn upcast_approval_outcome(
    task_id: Option<u64>,
    approved: bool,
    from_status: Option<StoredTaskStatus>,
    to_status: Option<StoredTaskStatus>,
) -> Result<StoredApprovalOutcome, PortError> {
    match (from_status, to_status) {
        (None, None) => Ok(StoredApprovalOutcome::Acknowledged { task_id, approved }),
        (Some(from_status), Some(to_status)) => {
            let task_id = task_id.ok_or_else(|| PortError::InternalContext {
                context: "v2 approval payload carries a transition without a task",
                source: "task_id is null".to_string(),
            })?;
            Ok(StoredApprovalOutcome::TaskTransition {
                task_id,
                approved,
                from_status,
                to_status,
            })
        }
        _ => Err(PortError::InternalContext {
            context: "v2 approval payload has mixed from/to status fields",
            source: "exactly one of from_status/to_status is present".to_string(),
        }),
    }
}

/// v2 encoding of `StoredEventPayload::ModelAgentProposalCompleted`: the
/// pre-v3 result struct with a separate terminal status field.
#[derive(Debug, Deserialize)]
pub(crate) struct StoredModelAgentProposalCompletedV2 {
    pub result: StoredModelAgentProposalResultV2,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StoredModelAgentProposalResultV2 {
    pub run_id: maestria_domain::HarnessRunId,
    pub correlation_id: u64,
    pub status: StoredModelAgentTerminalStatusV2,
    #[serde(default)]
    pub search: Option<maestria_domain::ModelAgentSearchResult>,
    #[serde(default)]
    pub harness: Option<maestria_domain::ModelAgentHarnessResult>,
    #[serde(default)]
    pub validation: Option<maestria_domain::ModelAgentValidationResult>,
    #[serde(default)]
    pub memory_candidate: Option<maestria_domain::ModelAgentMemoryResult>,
    #[serde(default)]
    pub error: Option<String>,
}

/// v2 terminal status: the pre-v3 enum serialized without case renaming.
#[derive(Debug, Deserialize)]
pub(crate) enum StoredModelAgentTerminalStatusV2 {
    Succeeded,
    Failed,
}

impl StoredModelAgentProposalCompletedV2 {
    pub(crate) fn into_v3(self) -> Result<StoredEventPayload, PortError> {
        let result = match self.result.status {
            StoredModelAgentTerminalStatusV2::Succeeded => {
                if self.result.error.is_some() {
                    return Err(PortError::InternalContext {
                        context: "v2 proposal result succeeds while carrying an error",
                        source: "error field is present with status Succeeded".to_string(),
                    });
                }
                maestria_domain::ModelAgentProposalResult::Succeeded {
                    run_id: self.result.run_id,
                    correlation_id: self.result.correlation_id,
                    search: self.result.search,
                    harness: self.result.harness,
                    validation: self.result.validation,
                    memory_candidate: self.result.memory_candidate,
                }
            }
            StoredModelAgentTerminalStatusV2::Failed => {
                let error = self
                    .result
                    .error
                    .ok_or_else(|| PortError::InternalContext {
                        context: "v2 proposal result fails without an error",
                        source: "error field is missing with status Failed".to_string(),
                    })?;
                maestria_domain::ModelAgentProposalResult::Failed {
                    run_id: self.result.run_id,
                    correlation_id: self.result.correlation_id,
                    error,
                }
            }
        };
        Ok(StoredEventPayload::ModelAgentProposalCompleted { result })
    }
}
