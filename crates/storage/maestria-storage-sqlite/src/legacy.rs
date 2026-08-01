use maestria_ports::PortError;

use crate::{
    payloads::{LegacyStoredEventPayload, StoredEventPayload},
    sqlite_store::json_error,
};

/// Decode a stored event payload by its row encoding version.
///
/// Version 1 is the pre-`payload_version` encoding (upcast through
/// `LegacyStoredEventPayload`), version 2 is the first versioned encoding,
/// and version 3 reshapes the approval and model-agent proposal payloads.
/// Domain-event rows are immutable: older encodings are upcast in memory
/// rather than rewritten.
pub(crate) fn decode_stored_payload(
    value: serde_json::Value,
    payload_version: i64,
) -> Result<StoredEventPayload, PortError> {
    match payload_version {
        1 => {
            let legacy: LegacyStoredEventPayload =
                serde_json::from_value(value).map_err(json_error)?;
            legacy.into_v2()
        }
        2 => match value.get("event_kind").and_then(serde_json::Value::as_str) {
            Some("approval_recorded") => {
                let v2: crate::payloads::payload_v2::StoredApprovalRecordedV2 =
                    serde_json::from_value(value).map_err(json_error)?;
                v2.into_v3()
            }
            Some("model_agent_proposal_completed") => {
                let v2: crate::payloads::payload_v2::StoredModelAgentProposalCompletedV2 =
                    serde_json::from_value(value).map_err(json_error)?;
                v2.into_v3()
            }
            _ => serde_json::from_value::<StoredEventPayload>(value).map_err(json_error),
        },
        3 => serde_json::from_value::<StoredEventPayload>(value).map_err(json_error),
        other => Err(PortError::InternalContext {
            context: "unsupported payload version",
            source: other.to_string(),
        }),
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
