use maestria_domain::{DomainEventEnvelope, EventId, SearchTraceId};
use maestria_ports::PortError;
use rusqlite::Row;

use crate::{
    payloads::StoredEventPayload,
    sqlite_store::{i64_to_u64, json_error, optional_i64_to_u64, to_port_error},
};

#[derive(Debug, Clone)]
pub(super) struct StoredEvent {
    pub(crate) id: u64,
    pub(crate) kind: &'static str,
    pub(crate) artifact_id: Option<u64>,
    pub(crate) payload_json: String,
    pub(crate) payload_version: i64,
}

impl StoredEvent {
    pub(super) fn from_domain(envelope: &DomainEventEnvelope) -> Result<Self, PortError> {
        let payload = StoredEventPayload::from_domain(&envelope.event)?;
        Ok(Self {
            id: envelope.id.value(),
            kind: payload.kind()?,
            artifact_id: payload.filter_artifact_id(),
            payload_json: serde_json::to_string(&payload).map_err(json_error)?,
            payload_version: crate::payloads::CURRENT_PAYLOAD_VERSION,
        })
    }
    pub(super) fn raw_search_trace(
        &self,
    ) -> Result<Option<maestria_domain::SearchTraceId>, PortError> {
        if self.kind != "search_knowledge_completed" {
            return Ok(None);
        }
        let payload: StoredEventPayload =
            serde_json::from_str(&self.payload_json).map_err(json_error)?;
        let StoredEventPayload::SearchKnowledgeCompleted { outcome, .. } = payload else {
            return Err(PortError::InternalContext {
                context: "stored search completion payload",
                source: "event kind does not match payload variant".to_string(),
            });
        };
        Ok(Some(SearchTraceId::new(outcome.trace)))
    }

    pub(super) fn into_domain(self) -> Result<DomainEventEnvelope, PortError> {
        let payload: StoredEventPayload =
            serde_json::from_str(&self.payload_json).map_err(json_error)?;
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
            event,
        })
    }
}

pub(super) fn read_stored_event(row: &Row<'_>) -> Result<StoredEvent, PortError> {
    let kind_ref = row
        .get_ref(1)
        .map_err(to_port_error)?
        .as_str()
        .map_err(|error| PortError::internal("decode event kind", error.to_string()))?;
    Ok(StoredEvent {
        id: i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?,
        kind: leaked_kind(kind_ref)?,
        artifact_id: optional_i64_to_u64(row.get::<_, Option<i64>>(2).map_err(to_port_error)?)?,
        payload_json: row.get::<_, String>(3).map_err(to_port_error)?,
        payload_version: row.get::<_, i64>(4).map_err(to_port_error)?,
    })
}

pub(super) fn leaked_kind(kind: &str) -> Result<&'static str, PortError> {
    match kind {
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
        "notebook_created" => Ok("notebook_created"),
        "notebook_renamed" => Ok("notebook_renamed"),
        "notebook_deleted" => Ok("notebook_deleted"),
        "notebook_source_attached" => Ok("notebook_source_attached"),
        "notebook_source_detached" => Ok("notebook_source_detached"),
        "notebook_draft_saved" => Ok("notebook_draft_saved"),
        "notebook_draft_deleted" => Ok("notebook_draft_deleted"),
        "realm_read_grant_issued" => Ok("realm_read_grant_issued"),
        "realm_read_grant_revoked" => Ok("realm_read_grant_revoked"),
        "federated_read_access_recorded" => Ok("federated_read_access_recorded"),
        other => Err(PortError::InternalContext {
            context: "unknown stored event kind",
            source: other.to_string(),
        }),
    }
}
