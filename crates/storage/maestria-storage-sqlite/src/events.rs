use maestria_domain::{DomainEventEnvelope, EventId, SequenceNumber};
use maestria_ports::PortError;
use rusqlite::Row;

use crate::{
    payloads::{LegacyStoredEventPayload, StoredEventPayload},
    sqlite_store::{i64_to_u64, json_error, optional_i64_to_u64, to_port_error},
};

#[derive(Debug, Clone)]
pub(super) struct StoredEvent {
    pub(crate) id: u64,
    pub(crate) sequence: u64,
    pub(crate) kind: &'static str,
    pub(crate) artifact_id: Option<u64>,
    pub(crate) payload_json: String,
    pub(crate) payload_version: i64,
    pub(crate) legacy_approval_id: Option<u64>,
}

impl StoredEvent {
    pub(super) fn from_domain(envelope: &DomainEventEnvelope) -> Result<Self, PortError> {
        let payload = StoredEventPayload::from_domain(&envelope.event)?;
        Ok(Self {
            id: envelope.id.value(),
            sequence: envelope.sequence.value(),
            kind: payload.kind()?,
            artifact_id: payload.filter_artifact_id(),
            payload_json: serde_json::to_string(&payload).map_err(json_error)?,
            payload_version: 2,
            legacy_approval_id: None,
        })
    }
    pub(super) fn raw_search_trace(
        &self,
    ) -> Result<Option<maestria_domain::SearchTraceId>, PortError> {
        if self.kind != "search_knowledge_completed" {
            return Ok(None);
        }
        let value = upcast_legacy_approval_id(&self.payload_json, self.legacy_approval_id)?;
        let payload: StoredEventPayload = serde_json::from_value(value).map_err(json_error)?;
        let StoredEventPayload::SearchKnowledgeCompleted { outcome, .. } = payload else {
            return Err(PortError::InternalContext {
                context: "stored search completion payload",
                source: "event kind does not match payload variant".to_string(),
            });
        };
        Ok(Some(outcome.trace))
    }

    pub(super) fn into_domain(self) -> Result<DomainEventEnvelope, PortError> {
        let value = upcast_legacy_approval_id(&self.payload_json, self.legacy_approval_id)?;
        let payload = match self.payload_version {
            1 => {
                let legacy: LegacyStoredEventPayload =
                    serde_json::from_value(value).map_err(json_error)?;
                legacy.into_v2()
            }
            2 => serde_json::from_value::<StoredEventPayload>(value).map_err(json_error),
            other => {
                return Err(PortError::InternalContext {
                    context: "unsupported payload version",
                    source: other.to_string(),
                });
            }
        }?;
        let payload_kind = payload.kind()?;
        if payload_kind != self.kind {
            return Err(PortError::InternalContext {
                context: "stored event kind mismatch",
                source: format!("column {}, payload {}", self.kind, payload_kind),
            });
        }
        if payload.filter_artifact_id() != self.artifact_id {
            return Err(PortError::InternalContext {
                context: "stored event artifact identity mismatch",
                source: "artifact_id column does not match payload".to_string(),
            });
        }
        let mut event = payload.into_domain()?;
        if let maestria_domain::DomainEvent::SearchKnowledgeCompleted { outcome, .. } = &mut event {
            outcome.canonicalize_score_provenance().map_err(|error| {
                PortError::InternalContext {
                    context: "canonicalize retrieval score provenance during replay",
                    source: error.to_string(),
                }
            })?;
        }
        Ok(DomainEventEnvelope {
            id: EventId::new(self.id),
            sequence: SequenceNumber::new(self.sequence),
            event,
        })
    }
}

/// Upcast legacy approval payloads in memory without rewriting their stored
/// bytes. Legacy identities come from the durable event-to-approval mapping;
/// an event row id is never treated as an approval id.
pub(crate) fn upcast_legacy_approval_id(
    payload_json: &str,
    mapped_approval_id: Option<u64>,
) -> Result<serde_json::Value, PortError> {
    let mut value: serde_json::Value = serde_json::from_str(payload_json).map_err(json_error)?;
    let Some(object) = value.as_object_mut() else {
        return Ok(value);
    };
    let is_approval =
        object.get("event_kind").and_then(serde_json::Value::as_str) == Some("approval_recorded");
    if !is_approval {
        return Ok(value);
    }
    let payload_approval_id = object
        .get("approval_id")
        .and_then(serde_json::Value::as_u64);
    if payload_approval_id.is_none()
        && object
            .get("approval_id")
            .is_some_and(|value| !value.is_null())
    {
        return Err(PortError::Conflict {
            message: "approval event contains an invalid approval identity".to_string(),
        });
    }
    match (payload_approval_id, mapped_approval_id) {
        (None, Some(id)) => {
            object.insert("approval_id".to_string(), serde_json::Value::from(id));
        }
        (None, None) => {
            return Err(PortError::Conflict {
                message: "legacy approval event has no durable event-to-approval mapping"
                    .to_string(),
            });
        }
        (Some(payload_id), Some(mapped_id)) if payload_id != mapped_id => {
            return Err(PortError::Conflict {
                message: "approval event payload conflicts with durable event-to-approval mapping"
                    .to_string(),
            });
        }
        _ => {}
    }
    Ok(value)
}

pub(super) fn read_stored_event(row: &Row<'_>) -> Result<StoredEvent, PortError> {
    Ok(StoredEvent {
        id: i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?,
        sequence: i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?,
        kind: leaked_kind(row.get::<_, String>(2).map_err(to_port_error)?)?,
        artifact_id: optional_i64_to_u64(row.get::<_, Option<i64>>(3).map_err(to_port_error)?)?,
        payload_json: row.get::<_, String>(4).map_err(to_port_error)?,
        payload_version: row.get::<_, i64>(5).map_err(to_port_error)?,
        legacy_approval_id: row
            .get::<_, Option<i64>>(6)
            .map_err(to_port_error)?
            .map(u64::try_from)
            .transpose()
            .map_err(|_| PortError::InternalContext {
                context: "decode mapped legacy approval id",
                source: "approval id is negative".to_string(),
            })?,
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
        "task_evidence_linked" => Ok("task_evidence_linked"),
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
        "search_executed" => Ok("search_executed"),
        "search_knowledge_completed" => Ok("search_knowledge_completed"),
        "model_agent_proposal_requested" => Ok("model_agent_proposal_requested"),
        "model_agent_proposal_completed" => Ok("model_agent_proposal_completed"),
        "pending_index" => Ok("pending_index"),
        "full_text_indexed" => Ok("full_text_indexed"),
        "artifact_indexed" => Ok("artifact_indexed"),
        "parser_started" => Ok("parser_started"),
        "ocr_requested" => Ok("ocr_requested"),
        "ocr_completed" => Ok("ocr_completed"),
        "ocr_failed" => Ok("ocr_failed"),
        "document_tree_captured" => Ok("document_tree_captured"),
        "index_generation_started" => Ok("index_generation_started"),
        "index_generation_transitioned" => Ok("index_generation_transitioned"),
        "source_became_stale" => Ok("source_became_stale"),
        other => Err(PortError::InternalContext {
            context: "unknown stored event kind",
            source: other.to_string(),
        }),
    }
}
