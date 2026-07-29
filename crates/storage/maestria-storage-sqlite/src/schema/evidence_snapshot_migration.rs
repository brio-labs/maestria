use super::evidence_snapshot_payload::migrate_kind_value;
use maestria_domain::ContentHash;
use maestria_ports::PortError;
use rusqlite::{Connection, OptionalExtension};

use crate::sqlite_store::to_port_error;

pub(super) fn migrate_evidence_snapshots_v10(connection: &Connection) -> Result<(), PortError> {
    migrate_evidence_rows(connection)?;
    migrate_evidence_recorded_events(connection)
}

fn migrate_evidence_rows(connection: &Connection) -> Result<(), PortError> {
    let mut statement = connection
        .prepare("SELECT id, artifact_id, kind_json FROM evidence ORDER BY id ASC")
        .map_err(to_port_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(to_port_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(to_port_error)?;
    drop(statement);

    for (id, artifact_id, kind_json) in rows {
        let kind_value: serde_json::Value =
            serde_json::from_str(&kind_json).map_err(|error| PortError::InternalContext {
                context: "migrate evidence snapshot",
                source: format!("evidence row {id}: invalid JSON: {error}"),
            })?;
        let owner_hash = if kind_requires_owner_hash(&kind_value) {
            Some(
                load_owner_content_hash(connection, artifact_id).map_err(|source| {
                    PortError::InternalContext {
                        context: "migrate evidence snapshot",
                        source: format!("evidence row {id}: {source}"),
                    }
                })?,
            )
        } else {
            None
        };
        let migrated = migrate_kind_value(kind_value, owner_hash.as_deref()).map_err(|source| {
            PortError::InternalContext {
                context: "migrate evidence snapshot",
                source: format!("evidence row {id}: {source}"),
            }
        })?;
        let migrated =
            serde_json::to_string(&migrated).map_err(|error| PortError::InternalContext {
                context: "migrate evidence snapshot",
                source: format!("evidence row {id}: serialize canonical kind: {error}"),
            })?;
        if migrated != kind_json {
            connection
                .execute(
                    "UPDATE evidence SET kind_json = ?1 WHERE id = ?2",
                    rusqlite::params![migrated, id],
                )
                .map_err(to_port_error)?;
        }
    }
    Ok(())
}

fn migrate_evidence_recorded_events(connection: &Connection) -> Result<(), PortError> {
    let mut statement = connection
        .prepare(
            "SELECT id, sequence, payload_json
             FROM domain_events
             WHERE event_kind = 'evidence_recorded' AND payload_version = 2
             ORDER BY sequence ASC",
        )
        .map_err(to_port_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(to_port_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(to_port_error)?;
    drop(statement);

    for (id, sequence, payload_json) in rows {
        let migrated =
            migrate_event_payload(connection, sequence, &payload_json).map_err(|source| {
                PortError::InternalContext {
                    context: "migrate evidence recorded snapshot",
                    source: format!("domain event {id}: {source}"),
                }
            })?;
        if migrated != payload_json {
            connection
                .execute(
                    "UPDATE domain_events
                     SET payload_json = ?1, payload_version = 2
                     WHERE id = ?2",
                    rusqlite::params![migrated, id],
                )
                .map_err(to_port_error)?;
        }
    }
    Ok(())
}

fn migrate_event_payload(
    connection: &Connection,
    sequence: i64,
    payload_json: &str,
) -> Result<String, String> {
    let mut payload: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|error| format!("invalid JSON: {error}"))?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "event payload must be a JSON object".to_string())?;
    if object.get("event_kind").and_then(serde_json::Value::as_str) != Some("evidence_recorded") {
        return Err("payload event_kind is not evidence_recorded".to_string());
    }
    let evidence_kind = object
        .get("evidence_kind")
        .cloned()
        .ok_or_else(|| "payload is missing evidence_kind".to_string())?;
    let owner_hash = if kind_requires_owner_hash(&evidence_kind) {
        let artifact_id = object
            .get("artifact_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "payload artifact_id is missing or invalid".to_string())?;
        Some(load_historical_content_hash(
            connection,
            i64::try_from(artifact_id)
                .map_err(|_| "payload artifact_id exceeds sqlite integer range".to_string())?,
            sequence,
        )?)
    } else {
        None
    };
    object.insert(
        "evidence_kind".to_string(),
        migrate_kind_value(evidence_kind, owner_hash.as_deref())?,
    );
    serde_json::to_string(&payload).map_err(|error| format!("serialize migrated payload: {error}"))
}
fn kind_requires_owner_hash(value: &serde_json::Value) -> bool {
    matches!(
        value.get("kind").and_then(serde_json::Value::as_str),
        Some("pdf_span" | "pdf_region")
    ) && !value
        .get("snapshot")
        .is_some_and(serde_json::Value::is_object)
}
fn load_historical_content_hash(
    connection: &Connection,
    artifact_id: i64,
    evidence_sequence: i64,
) -> Result<String, String> {
    if artifact_id < 0 {
        return Err("owning artifact identity is negative".to_string());
    }
    let mut statement = connection
        .prepare(
            "SELECT event_kind, payload_json, payload_version
             FROM domain_events
             WHERE artifact_id = ?1
               AND sequence < ?2
               AND event_kind IN (
                   'pending_index',
                   'parser_started',
                   'source_became_stale',
                   'document_tree_captured'
               )
             ORDER BY sequence DESC",
        )
        .map_err(|error| format!("load preceding hash-bearing events: {error}"))?;
    let rows = statement
        .query_map(rusqlite::params![artifact_id, evidence_sequence], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|error| format!("load preceding hash-bearing events: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("load preceding hash-bearing events: {error}"))?;
    drop(statement);

    let Some((event_kind, payload_json, _payload_version)) = rows.into_iter().next() else {
        return Err(format!(
            "no preceding hash-bearing event for artifact {artifact_id} before sequence {evidence_sequence}"
        ));
    };
    let payload: serde_json::Value = serde_json::from_str(&payload_json)
        .map_err(|error| format!("preceding {event_kind} payload has invalid JSON: {error}"))?;
    if payload
        .get("event_kind")
        .and_then(serde_json::Value::as_str)
        != Some(event_kind.as_str())
    {
        return Err(format!(
            "preceding {event_kind} payload has inconsistent event_kind"
        ));
    }
    if payload
        .get("artifact_id")
        .and_then(serde_json::Value::as_u64)
        != u64::try_from(artifact_id).ok()
    {
        return Err(format!(
            "preceding {event_kind} payload has inconsistent artifact_id"
        ));
    }
    let hash = payload
        .get("content_hash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("preceding {event_kind} payload is missing content_hash"))?;
    ContentHash::new(hash.to_owned())
        .map_err(|error| format!("preceding {event_kind} has invalid content hash: {error}"))?;
    Ok(hash.to_owned())
}

fn load_owner_content_hash(connection: &Connection, artifact_id: i64) -> Result<String, String> {
    if artifact_id < 0 {
        return Err("owning artifact identity is negative".to_string());
    }
    let hash: Option<String> = connection
        .query_row(
            "SELECT content_hash FROM artifacts WHERE id = ?1",
            [artifact_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("load owning artifact {artifact_id} content hash: {error}"))?
        .flatten();
    let hash = hash
        .ok_or_else(|| format!("owning artifact {artifact_id} content hash is missing or null"))?;
    ContentHash::new(hash.clone()).map_err(|error| {
        format!("owning artifact {artifact_id} has invalid content hash: {error}")
    })?;
    Ok(hash)
}
