use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

use super::event_payloads::StoredEventPayload;
use super::evidence_payloads::{
    StoredClaimStatus, StoredEvidenceKind, StoredTaskPriority, StoredTaskStatus,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event_kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LegacyStoredEventPayload {
    ArtifactRegistered {
        artifact_id: u64,
        title: String,
    },
    ChunkRegistered {
        chunk_id: u64,
        artifact_id: u64,
        order: u32,
    },
    CardCreated {
        card_id: u64,
        artifact_id: u64,
    },
    ClaimCreated {
        claim_id: u64,
        artifact_id: u64,
    },
    EvidenceRecorded {
        evidence_id: u64,
        evidence_kind: StoredEvidenceKind,
        artifact_id: u64,
        claim_id: Option<u64>,
    },
    TaskOpened {
        task_id: u64,
        title: String,
        priority: StoredTaskPriority,
    },
    TaskStatusChanged {
        task_id: u64,
        from: StoredTaskStatus,
        to: StoredTaskStatus,
    },
    TaskCompletionRecorded {
        task_id: u64,
        status: StoredTaskStatus,
        validation_report_id: u64,
    },
    TaskEvidenceLinked {
        task_id: u64,
        evidence_id: u64,
    },
    ClaimValidationUpdated {
        claim_id: u64,
        status: StoredClaimStatus,
    },
    ClaimEvidenceLinked {
        claim_id: u64,
        evidence_id: u64,
    },
    RelationCreated {
        relation_id: u64,
    },
    MemoryCandidateCreated {
        candidate_id: u64,
        claim_id: u64,
        evidence_ids: Vec<u64>,
        confidence_milli: u16,
    },
    MemoryPromoted {
        memory_id: u64,
        candidate_id: u64,
    },
    MemoryContradicted {
        memory_id: u64,
        contradicting_candidate_id: u64,
    },
    MemoryDeprecated {
        memory_id: u64,
    },
    MemorySuperseded {
        memory_id: u64,
        by_memory_id: u64,
    },
    ValidationReportCreated {
        report_id: u64,
        task_id: Option<u64>,
        passed: bool,
        warnings: Vec<String>,
    },
    UserIntentObserved {
        task_id: u64,
        title: String,
    },
    ArtifactParsed {
        artifact_id: u64,
        chunks_added: u32,
    },
    SearchCompleted {
        artifact_id: u64,
        cards_added: u32,
    },
    HarnessRunCompleted {
        task_id: Option<u64>,
        command: String,
        exit_code: i32,
    },
    ApprovalRecorded {
        approval_id: Option<u64>,
        task_id: u64,
        approved: bool,
        from_status: Option<StoredTaskStatus>,
        to_status: Option<StoredTaskStatus>,
    },
    TickObserved {
        at: u64,
    },
}

impl LegacyStoredEventPayload {
    pub(crate) fn into_v2(self) -> Result<StoredEventPayload, PortError> {
        match self {
            Self::ChunkRegistered { .. }
            | Self::CardCreated { .. }
            | Self::ClaimCreated { .. }
            | Self::EvidenceRecorded { .. }
            | Self::TaskOpened { .. }
            | Self::RelationCreated { .. } => self.into_v2_unsupported(),
            Self::MemoryCandidateCreated { .. }
            | Self::MemoryPromoted { .. }
            | Self::MemoryContradicted { .. }
            | Self::MemoryDeprecated { .. }
            | Self::MemorySuperseded { .. } => self.into_v2_memory(),
            Self::ArtifactRegistered { artifact_id, title } => {
                Ok(StoredEventPayload::ArtifactRegistered { artifact_id, title })
            }
            Self::TaskStatusChanged { task_id, from, to } => {
                Ok(StoredEventPayload::TaskStatusChanged { task_id, from, to })
            }
            Self::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => Ok(StoredEventPayload::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            }),
            Self::ClaimValidationUpdated { claim_id, status } => {
                Ok(StoredEventPayload::ClaimValidationUpdated { claim_id, status })
            }
            Self::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => Ok(StoredEventPayload::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            }),
            Self::TaskEvidenceLinked {
                task_id,
                evidence_id,
            } => Ok(StoredEventPayload::TaskEvidenceLinked {
                task_id,
                evidence_id,
            }),
            Self::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => Ok(StoredEventPayload::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            }),
            Self::UserIntentObserved { task_id, title } => {
                Ok(StoredEventPayload::UserIntentObserved { task_id, title })
            }
            Self::ArtifactParsed {
                artifact_id,
                chunks_added,
            } => Ok(StoredEventPayload::ArtifactParsed {
                artifact_id,
                chunks_added,
            }),
            Self::SearchCompleted {
                artifact_id,
                cards_added,
            } => Ok(StoredEventPayload::SearchCompleted {
                artifact_id,
                cards_added,
            }),
            Self::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            } => Ok(StoredEventPayload::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            }),
            Self::ApprovalRecorded {
                approval_id,
                task_id,
                approved,
                from_status,
                to_status,
            } => {
                let id = approval_id.ok_or_else(|| PortError::Internal {
                    message: "legacy ApprovalRecorded payload missing approval_id".into(),
                })?;
                let from = from_status.ok_or_else(|| PortError::Internal {
                    message: "legacy ApprovalRecorded payload missing from_status".into(),
                })?;
                let to = to_status.ok_or_else(|| PortError::Internal {
                    message: "legacy ApprovalRecorded payload missing to_status".into(),
                })?;
                Ok(StoredEventPayload::ApprovalRecorded {
                    approval_id: id,
                    task_id,
                    approved,
                    from_status: Some(from),
                    to_status: Some(to),
                })
            }
            Self::TickObserved { at } => Ok(StoredEventPayload::TickObserved { at }),
        }
    }

    fn unsupported_error(kind: &str, field: &str) -> PortError {
        PortError::InvalidInput {
            message: format!("V1 {kind} event is missing required field(s): {field}"),
        }
    }

    fn into_v2_unsupported(self) -> Result<StoredEventPayload, PortError> {
        match self {
            Self::ChunkRegistered { .. } => Err(Self::unsupported_error("ChunkRegistered", "text")),
            Self::CardCreated { .. } => {
                Err(Self::unsupported_error("CardCreated", "title and body"))
            }
            Self::ClaimCreated { .. } => Err(Self::unsupported_error(
                "ClaimCreated",
                "text and evidence_ids",
            )),
            Self::EvidenceRecorded { .. } => Err(Self::unsupported_error(
                "EvidenceRecorded",
                "excerpt and observed_at",
            )),
            Self::TaskOpened { .. } => Err(Self::unsupported_error("TaskOpened", "artifact_id")),
            Self::RelationCreated { .. } => Err(Self::unsupported_error(
                "RelationCreated",
                "source, kind, target, evidence_id, confidence_milli",
            )),
            _ => unreachable!("into_v2_unsupported called on supported variant"),
        }
    }

    fn into_v2_memory(self) -> Result<StoredEventPayload, PortError> {
        match self {
            Self::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => Ok(StoredEventPayload::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            }),
            Self::MemoryPromoted {
                memory_id,
                candidate_id,
            } => Ok(StoredEventPayload::MemoryPromoted {
                memory_id,
                candidate_id,
            }),
            Self::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => Ok(StoredEventPayload::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            }),
            Self::MemoryDeprecated { memory_id } => {
                Ok(StoredEventPayload::MemoryDeprecated { memory_id })
            }
            Self::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => Ok(StoredEventPayload::MemorySuperseded {
                memory_id,
                by_memory_id,
            }),
            _ => unreachable!("into_v2_memory called on non-memory variant"),
        }
    }
}
