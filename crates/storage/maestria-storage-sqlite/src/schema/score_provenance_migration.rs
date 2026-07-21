use std::collections::BTreeMap;

use maestria_domain::{EvidencePackReproducibilityRecord, SearchTraceId};
use maestria_ports::PortError;
use rusqlite::{Connection, params};

use super::{CURRENT_SCHEMA_VERSION, SchemaState, security_migration::validate_at_v8};
use crate::{json_error, payloads::StoredEventPayload, to_port_error};

const SEARCH_COMPLETED_KIND: &str = "search_knowledge_completed";
const SEARCH_EXECUTED_KIND: &str = "search_executed";

/// Canonicalizes persisted retrieval score provenance and remaps every stored
/// evidence-pack reference from the legacy trace identity to the v6 identity.
///
/// The migration is deliberately idempotent. A transaction either rewrites all
/// affected payloads and their references or leaves the database unchanged.
pub(super) fn migrate_score_provenance_v9(connection: &Connection) -> Result<(), PortError> {
    let completed_rows = load_payload_rows(connection, SEARCH_COMPLETED_KIND)?;
    let mut trace_remap = BTreeMap::<SearchTraceId, SearchTraceId>::new();

    for (id, payload_json) in completed_rows {
        let mut payload: StoredEventPayload =
            serde_json::from_str(&payload_json).map_err(json_error)?;
        let StoredEventPayload::SearchKnowledgeCompleted { outcome, .. } = &mut payload else {
            return Err(PortError::Internal {
                message: format!(
                    "stored {SEARCH_COMPLETED_KIND} row {id} has an incompatible payload variant"
                ),
            });
        };

        let old_trace = outcome.trace;
        outcome
            .canonicalize_score_provenance()
            .map_err(|error| PortError::Internal {
                message: format!(
                    "cannot migrate retrieval score provenance for event {id}: {error}"
                ),
            })?;
        let new_trace = outcome.trace;
        if let Some(previous) = trace_remap.insert(old_trace, new_trace)
            && previous != new_trace
        {
            return Err(PortError::Internal {
                message: format!(
                    "legacy search trace {old_trace} maps to conflicting v6 identities"
                ),
            });
        }

        let canonical = serde_json::to_string(&payload).map_err(json_error)?;
        reject_legacy_score_shape(id, &canonical)?;
        update_payload(connection, id, &payload_json, &canonical)?;
    }

    for (id, payload_json) in load_payload_rows(connection, SEARCH_EXECUTED_KIND)? {
        let mut payload: StoredEventPayload =
            serde_json::from_str(&payload_json).map_err(json_error)?;
        let StoredEventPayload::SearchExecuted { pack_metadata, .. } = &mut payload else {
            return Err(PortError::Internal {
                message: format!(
                    "stored {SEARCH_EXECUTED_KIND} row {id} has an incompatible payload variant"
                ),
            });
        };
        let Some(metadata) = pack_metadata.as_mut() else {
            continue;
        };
        if let Some(trace) = metadata.search_trace
            && let Some(replacement) = trace_remap.get(&trace)
        {
            metadata.search_trace = Some(*replacement);
        }
        if let EvidencePackReproducibilityRecord::Frozen(replay) = &mut metadata.reproducibility
            && let Some(replacement) = trace_remap.get(&replay.trace)
        {
            replay.trace = *replacement;
        }

        let canonical = serde_json::to_string(&payload).map_err(json_error)?;
        update_payload(connection, id, &payload_json, &canonical)?;
    }

    connection
        .execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}

pub(super) fn validate_at_v9(
    connection: &Connection,
    state: &SchemaState,
) -> Result<(), PortError> {
    validate_at_v8(connection, state)?;
    for (id, payload_json) in load_payload_rows(connection, SEARCH_COMPLETED_KIND)? {
        reject_legacy_score_shape(id, &payload_json)?;
        let payload: StoredEventPayload =
            serde_json::from_str(&payload_json).map_err(json_error)?;
        let StoredEventPayload::SearchKnowledgeCompleted { outcome, .. } = payload else {
            return Err(PortError::Internal {
                message: format!(
                    "stored {SEARCH_COMPLETED_KIND} row {id} has an incompatible payload variant"
                ),
            });
        };
        if outcome
            .evidence
            .iter()
            .chain(
                outcome
                    .conflicts
                    .iter()
                    .flat_map(|set| set.candidates.iter()),
            )
            .any(|candidate| {
                candidate.scores.schema_version != maestria_domain::RETRIEVAL_SCORE_SCHEMA_VERSION
            })
        {
            return Err(PortError::Internal {
                message: format!("search outcome event {id} has a non-canonical score schema"),
            });
        }
    }
    Ok(())
}

fn load_payload_rows(
    connection: &Connection,
    event_kind: &str,
) -> Result<Vec<(i64, String)>, PortError> {
    let mut statement = connection
        .prepare(
            "SELECT id, payload_json
             FROM domain_events
             WHERE event_kind = ?1 AND payload_version = 2
             ORDER BY sequence ASC",
        )
        .map_err(to_port_error)?;
    let rows = statement
        .query_map([event_kind], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(to_port_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(to_port_error)
}

fn update_payload(
    connection: &Connection,
    id: i64,
    previous: &str,
    canonical: &str,
) -> Result<(), PortError> {
    if previous == canonical {
        return Ok(());
    }
    let updated = connection
        .execute(
            "UPDATE domain_events SET payload_json = ?1 WHERE id = ?2 AND payload_json = ?3",
            params![canonical, id, previous],
        )
        .map_err(to_port_error)?;
    if updated != 1 {
        return Err(PortError::Conflict {
            message: format!("event {id} changed during score provenance migration"),
        });
    }
    Ok(())
}

fn reject_legacy_score_shape(id: i64, payload_json: &str) -> Result<(), PortError> {
    let value: serde_json::Value = serde_json::from_str(payload_json).map_err(json_error)?;
    if contains_legacy_score_key(&value) {
        return Err(PortError::Internal {
            message: format!(
                "search outcome event {id} still contains a legacy retrieval score field"
            ),
        });
    }
    Ok(())
}

fn contains_legacy_score_key(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => fields.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "bm25" | "semantic_similarity" | "score_micros"
            ) || contains_legacy_score_key(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_legacy_score_key),
        _ => false,
    }
}
