use maestria_domain::{DomainEventEnvelope, EventId, SequenceNumber};
use maestria_ports::PortError;
use rusqlite::Row;

use crate::{
    i64_to_u64, optional_i64_to_u64,
    payloads::{LegacyStoredEventPayload, StoredEventPayload},
};

#[derive(Debug)]
pub(super) struct StoredEvent {
    pub(crate) id: u64,
    pub(crate) sequence: u64,
    pub(crate) kind: &'static str,
    pub(crate) artifact_id: Option<u64>,
    pub(crate) payload_json: String,
    pub(crate) payload_version: i64,
}

impl StoredEvent {
    pub(super) fn from_domain(envelope: &DomainEventEnvelope) -> Result<Self, PortError> {
        let payload = StoredEventPayload::from_domain(&envelope.event);
        Ok(Self {
            id: envelope.id.value(),
            sequence: envelope.sequence.value(),
            kind: payload.kind(),
            artifact_id: payload.filter_artifact_id(),
            payload_json: serde_json::to_string(&payload).map_err(crate::json_error)?,
            payload_version: 2,
        })
    }

    pub(super) fn into_domain(self) -> Result<DomainEventEnvelope, PortError> {
        let payload = match self.payload_version {
            1 => {
                let legacy: LegacyStoredEventPayload =
                    serde_json::from_str(&self.payload_json).map_err(crate::json_error)?;
                legacy.into_v2()
            }
            2 => Ok(
                serde_json::from_str::<StoredEventPayload>(&self.payload_json)
                    .map_err(crate::json_error)?,
            ),
            other => {
                return Err(PortError::Internal {
                    message: format!("unsupported payload version {}", other),
                });
            }
        }?;
        if payload.kind() != self.kind {
            return Err(PortError::Internal {
                message: format!(
                    "stored event kind mismatch: column {}, payload {}",
                    self.kind,
                    payload.kind()
                ),
            });
        }
        if payload.filter_artifact_id() != self.artifact_id {
            return Err(PortError::Internal {
                message: "stored event artifact_id mismatch".to_string(),
            });
        }
        Ok(DomainEventEnvelope {
            id: EventId::new(self.id),
            sequence: SequenceNumber::new(self.sequence),
            event: payload.into_domain(),
        })
    }
}

pub(super) fn read_stored_event(row: &Row<'_>) -> Result<StoredEvent, PortError> {
    Ok(StoredEvent {
        id: i64_to_u64(row.get::<_, i64>(0).map_err(crate::to_port_error)?)?,
        sequence: i64_to_u64(row.get::<_, i64>(1).map_err(crate::to_port_error)?)?,
        kind: leaked_kind(row.get::<_, String>(2).map_err(crate::to_port_error)?)?,
        artifact_id: optional_i64_to_u64(
            row.get::<_, Option<i64>>(3).map_err(crate::to_port_error)?,
        )?,
        payload_json: row.get::<_, String>(4).map_err(crate::to_port_error)?,
        payload_version: row.get::<_, i64>(5).map_err(crate::to_port_error)?,
    })
}

pub(super) fn leaked_kind(kind: String) -> Result<&'static str, PortError> {
    match kind.as_str() {
        "artifact_registered" => Ok("artifact_registered"),
        "chunk_registered" => Ok("chunk_registered"),
        "card_created" => Ok("card_created"),
        "claim_created" => Ok("claim_created"),
        "evidence_recorded" => Ok("evidence_recorded"),
        "task_opened" => Ok("task_opened"),
        "task_status_changed" => Ok("task_status_changed"),
        "task_completion_recorded" => Ok("task_completion_recorded"),
        "claim_validation_updated" => Ok("claim_validation_updated"),
        "claim_evidence_linked" => Ok("claim_evidence_linked"),
        "relation_created" => Ok("relation_created"),
        "memory_candidate_created" => Ok("memory_candidate_created"),
        "memory_promoted" => Ok("memory_promoted"),
        "memory_contradicted" => Ok("memory_contradicted"),
        "memory_deprecated" => Ok("memory_deprecated"),
        "memory_superseded" => Ok("memory_superseded"),
        "validation_report_created" => Ok("validation_report_created"),
        "user_intent_observed" => Ok("user_intent_observed"),
        "artifact_parsed" => Ok("artifact_parsed"),
        "search_completed" => Ok("search_completed"),
        "harness_run_completed" => Ok("harness_run_completed"),
        "approval_recorded" => Ok("approval_recorded"),
        "tick_observed" => Ok("tick_observed"),
        "pending_index" => Ok("pending_index"),
        "full_text_indexed" => Ok("full_text_indexed"),
        "artifact_indexed" => Ok("artifact_indexed"),
        other => Err(PortError::Internal {
            message: format!("unknown stored event kind {other}"),
        }),
    }
}
