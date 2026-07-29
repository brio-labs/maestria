use maestria_ports::PortError;
use rusqlite::Connection;

use super::{CURRENT_SCHEMA_VERSION, SchemaState, security_migration::validate_at_v8};
use crate::{
    payloads::StoredEventPayload,
    sqlite_store::{json_error, to_port_error},
};

const SEARCH_COMPLETED_KIND: &str = "search_knowledge_completed";
const SEARCH_EXECUTED_KIND: &str = "search_executed";

/// Validate retrieval score provenance without rewriting domain-event rows.
///
/// Legacy score DTOs are upcast in memory during scan/replay, preserving the
/// exact persisted payload bytes and their original payload version.
pub(super) fn migrate_score_provenance_v9(connection: &Connection) -> Result<(), PortError> {
    // Validate and upcast in memory. Domain-event payload bytes are immutable;
    // replay performs the canonical conversion through the versioned DTO.
    for (id, payload_json) in load_payload_rows(connection, SEARCH_COMPLETED_KIND)? {
        let mut payload: StoredEventPayload =
            serde_json::from_str(&payload_json).map_err(json_error)?;
        let StoredEventPayload::SearchKnowledgeCompleted { outcome, .. } = &mut payload else {
            return Err(PortError::InternalContext {
                context: "migrate search completion payload",
                source: format!(
                    "stored {SEARCH_COMPLETED_KIND} row {id} has an incompatible payload variant"
                ),
            });
        };
        outcome
            .canonicalize_score_provenance()
            .map_err(|error| PortError::InternalContext {
                context: "canonicalize retrieval score provenance",
                source: format!("cannot validate event {id}: {error}"),
            })?;
    }

    for (id, payload_json) in load_payload_rows(connection, SEARCH_EXECUTED_KIND)? {
        let payload: StoredEventPayload =
            serde_json::from_str(&payload_json).map_err(json_error)?;
        if !matches!(payload, StoredEventPayload::SearchExecuted { .. }) {
            return Err(PortError::InternalContext {
                context: "stored search_executed payload has incompatible variant",
                source: id.to_string(),
            });
        }
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
        let mut payload: StoredEventPayload =
            serde_json::from_str(&payload_json).map_err(json_error)?;
        let StoredEventPayload::SearchKnowledgeCompleted { outcome, .. } = &mut payload else {
            return Err(PortError::InternalContext {
                context: "validate search completion payload",
                source: format!(
                    "stored {SEARCH_COMPLETED_KIND} row {id} has an incompatible payload variant"
                ),
            });
        };
        outcome
            .canonicalize_score_provenance()
            .map_err(|error| PortError::InternalContext {
                context: "validate retrieval score schema",
                source: format!("search outcome event {id}: {error}"),
            })?;
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
