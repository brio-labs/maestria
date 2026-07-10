#![forbid(unsafe_code)]

//! SQLite-backed metadata and event-log adapter for Maestria.
//!
//! This crate intentionally keeps storage serialization at the port boundary:
//! domain types do not implement or depend on serde.

use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Mutex, MutexGuard},
};

use maestria_domain::{
    Artifact, ArtifactId, BlobId, Card, CardId, Chunk, ChunkId, ClaimId, ClaimStatus, ContentRange,
    DomainEvent, DomainEventEnvelope, EventId, Evidence, EvidenceId, EvidenceKind, HarnessRunId,
    LogicalTick, OutputStream, SequenceNumber, TaskId, TaskPriority, TaskStatus,
    ValidationReportId,
};
use maestria_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EventFilter, EventLog, EvidenceRepository,
    PortError,
};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

const CURRENT_SCHEMA_VERSION: i64 = 2;

/// SQLite-backed implementation of artifact metadata and the domain event log.
pub struct SqliteStore {
    connection: Mutex<Connection>,
}

impl SqliteStore {
    /// Open a SQLite database file and apply idempotent schema migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PortError> {
        let mut connection = Connection::open(path).map_err(to_port_error)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Open an in-memory SQLite database and apply idempotent schema migrations.
    ///
    /// Useful for crate-local tests and short-lived adapters.
    pub fn in_memory() -> Result<Self, PortError> {
        let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, PortError> {
        self.connection.lock().map_err(|_| PortError::Internal {
            message: "sqlite connection lock poisoned".to_string(),
        })
    }
}

impl ArtifactRepository for SqliteStore {
    fn get(&self, artifact_id: ArtifactId) -> Result<Option<Artifact>, PortError> {
        let connection = self.lock()?;
        let title = connection
            .query_row(
                "SELECT title FROM artifacts WHERE id = ?1",
                params![u64_to_i64(artifact_id.value())?],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(to_port_error)?;

        let Some(title) = title else {
            return Ok(None);
        };

        Ok(Some(Artifact {
            id: artifact_id,
            title,
            chunk_ids: load_id_set(&connection, "artifact_chunks", artifact_id, ChunkId::new)?,
            card_ids: load_id_set(&connection, "artifact_cards", artifact_id, CardId::new)?,
            claim_ids: load_id_set(&connection, "artifact_claims", artifact_id, ClaimId::new)?,
            evidence_ids: load_id_set(
                &connection,
                "artifact_evidences",
                artifact_id,
                EvidenceId::new,
            )?,
        }))
    }

    fn put(&self, artifact: Artifact) -> Result<(), PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        transaction
            .execute(
                "INSERT INTO artifacts (id, title) VALUES (?1, ?2)\n                 ON CONFLICT(id) DO UPDATE SET title = excluded.title",
                params![u64_to_i64(artifact.id.value())?, artifact.title],
            )
            .map_err(to_port_error)?;

        replace_id_set(
            &transaction,
            "artifact_chunks",
            artifact.id,
            artifact.chunk_ids.iter().map(|id| id.value()),
        )?;
        replace_id_set(
            &transaction,
            "artifact_cards",
            artifact.id,
            artifact.card_ids.iter().map(|id| id.value()),
        )?;
        replace_id_set(
            &transaction,
            "artifact_claims",
            artifact.id,
            artifact.claim_ids.iter().map(|id| id.value()),
        )?;
        replace_id_set(
            &transaction,
            "artifact_evidences",
            artifact.id,
            artifact.evidence_ids.iter().map(|id| id.value()),
        )?;

        transaction.commit().map_err(to_port_error)
    }
}

impl ChunkRepository for SqliteStore {
    fn get(&self, chunk_id: ChunkId) -> Result<Option<Chunk>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, artifact_id, chunk_order, text FROM chunks WHERE id = ?1")
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(chunk_id.value())?])
            .map_err(to_port_error)?;
        rows.next()
            .map_err(to_port_error)?
            .map(read_chunk)
            .transpose()
    }

    fn put(&self, chunk: Chunk) -> Result<(), PortError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO chunks (id, artifact_id, chunk_order, text) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                     artifact_id = excluded.artifact_id,
                     chunk_order = excluded.chunk_order,
                     text = excluded.text",
                params![
                    u64_to_i64(chunk.id.value())?,
                    u64_to_i64(chunk.artifact_id.value())?,
                    i64::from(chunk.order),
                    chunk.text,
                ],
            )
            .map(|_| ())
            .map_err(to_port_error)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Chunk>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, chunk_order, text
                 FROM chunks
                 WHERE artifact_id = ?1
                 ORDER BY chunk_order ASC, id ASC",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(artifact_id.value())?])
            .map_err(to_port_error)?;
        let mut chunks = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            chunks.push(read_chunk(row)?);
        }
        Ok(chunks)
    }
}

impl CardRepository for SqliteStore {
    fn get(&self, card_id: CardId) -> Result<Option<Card>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, artifact_id, title, body FROM cards WHERE id = ?1")
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(card_id.value())?])
            .map_err(to_port_error)?;
        rows.next()
            .map_err(to_port_error)?
            .map(|row| read_card(row, &connection))
            .transpose()
    }

    fn put(&self, card: Card) -> Result<(), PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        transaction
            .execute(
                "INSERT INTO cards (id, artifact_id, title, body) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                     artifact_id = excluded.artifact_id,
                     title = excluded.title,
                     body = excluded.body",
                params![
                    u64_to_i64(card.id.value())?,
                    u64_to_i64(card.artifact_id.value())?,
                    card.title,
                    card.body,
                ],
            )
            .map_err(to_port_error)?;
        replace_card_claims(
            &transaction,
            card.id,
            card.claim_ids.iter().map(|id| id.value()),
        )?;
        transaction.commit().map_err(to_port_error)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Card>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, title, body
                 FROM cards
                 WHERE artifact_id = ?1
                 ORDER BY id ASC",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(artifact_id.value())?])
            .map_err(to_port_error)?;
        let mut cards = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            cards.push(read_card(row, &connection)?);
        }
        Ok(cards)
    }
}

impl EvidenceRepository for SqliteStore {
    fn get(&self, evidence_id: EvidenceId) -> Result<Option<Evidence>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, claim_id, kind_json, excerpt, observed_at
                 FROM evidence
                 WHERE id = ?1",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(evidence_id.value())?])
            .map_err(to_port_error)?;
        rows.next()
            .map_err(to_port_error)?
            .map(read_evidence)
            .transpose()
    }

    fn put(&self, evidence: Evidence) -> Result<(), PortError> {
        let kind_json = serde_json::to_string(&StoredEvidenceKind::from_domain(&evidence.kind))
            .map_err(json_error)?;
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO evidence
                     (id, artifact_id, claim_id, kind_json, excerpt, observed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                     artifact_id = excluded.artifact_id,
                     claim_id = excluded.claim_id,
                     kind_json = excluded.kind_json,
                     excerpt = excluded.excerpt,
                     observed_at = excluded.observed_at",
                params![
                    u64_to_i64(evidence.id.value())?,
                    u64_to_i64(evidence.artifact_id.value())?,
                    optional_u64_to_i64(evidence.claim_id.map(|id| id.value()))?,
                    kind_json,
                    evidence.excerpt,
                    u64_to_i64(evidence.observed_at.value())?,
                ],
            )
            .map(|_| ())
            .map_err(to_port_error)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, claim_id, kind_json, excerpt, observed_at
                 FROM evidence
                 WHERE artifact_id = ?1
                 ORDER BY id ASC",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(artifact_id.value())?])
            .map_err(to_port_error)?;
        let mut evidences = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            evidences.push(read_evidence(row)?);
        }
        Ok(evidences)
    }
}

impl EventLog for SqliteStore {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        let record = StoredEvent::from_domain(&event)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        let (count, max_id, max_sequence, mismatched): (i64, Option<i64>, Option<i64>, i64) =
            transaction
                .query_row(
                    "SELECT COUNT(*), MAX(id), MAX(sequence),
                            COALESCE(SUM(CASE WHEN id != sequence OR id < 1 THEN 1 ELSE 0 END), 0)
                     FROM domain_events",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(to_port_error)?;
        let count = u64::try_from(count).map_err(|_| PortError::Internal {
            message: "stored event count is negative".to_string(),
        })?;
        if count > 0 {
            if mismatched != 0 {
                return Err(PortError::Conflict {
                    message: "stored event log has mismatched ids and sequences".to_string(),
                });
            }
            let max_id = max_id.ok_or_else(|| PortError::Internal {
                message: "stored event log has no maximum id".to_string(),
            })?;
            let max_sequence = max_sequence.ok_or_else(|| PortError::Internal {
                message: "stored event log has no maximum sequence".to_string(),
            })?;
            let max_id = i64_to_u64(max_id)?;
            let max_sequence = i64_to_u64(max_sequence)?;
            if max_id != count || max_sequence != count {
                return Err(PortError::Conflict {
                    message: "stored event log is not contiguous".to_string(),
                });
            }
        }
        let expected_sequence = count + 1;
        if record.id != expected_sequence || record.sequence != expected_sequence {
            return Err(PortError::Conflict {
                message: format!(
                    "expected event id/sequence {expected_sequence}, got id {}, sequence {}",
                    record.id, record.sequence
                ),
            });
        }
        transaction
            .execute(
                "INSERT INTO domain_events \
                     (id, sequence, event_kind, artifact_id, payload_json, payload_version)\n                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    u64_to_i64(record.id)?,
                    u64_to_i64(record.sequence)?,
                    record.kind,
                    optional_u64_to_i64(record.artifact_id)?,
                    record.payload_json,
                    record.payload_version,
                ],
            )
            .map_err(map_append_error)?;
        transaction.commit().map_err(to_port_error)
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        let connection = self.lock()?;
        let mut events = Vec::new();

        if let Some(artifact_id) = filter.artifact_id {
            let mut statement = connection
                .prepare(
                    "SELECT id, sequence, event_kind, artifact_id, payload_json, payload_version\n                     FROM domain_events\n                     WHERE artifact_id = ?1\n                     ORDER BY sequence ASC",
                )
                .map_err(to_port_error)?;
            let mut rows = statement
                .query(params![u64_to_i64(artifact_id.value())?])
                .map_err(to_port_error)?;
            while let Some(row) = rows.next().map_err(to_port_error)? {
                events.push(read_stored_event(row)?.into_domain()?);
            }
        } else {
            let mut statement = connection
                .prepare(
                    "SELECT id, sequence, event_kind, artifact_id, payload_json, payload_version\n                     FROM domain_events\n                     ORDER BY sequence ASC",
                )
                .map_err(to_port_error)?;
            let mut rows = statement.query([]).map_err(to_port_error)?;
            while let Some(row) = rows.next().map_err(to_port_error)? {
                events.push(read_stored_event(row)?.into_domain()?);
            }
        }

        Ok(events)
    }
}

fn migrate(connection: &mut Connection) -> Result<(), PortError> {
    let transaction = connection.transaction().map_err(to_port_error)?;

    let had_schema_version_table = table_exists(&transaction, "schema_version")?;
    let had_domain_events_table = table_exists(&transaction, "domain_events")?;
    let had_payload_version_column = if had_domain_events_table {
        table_has_column(&transaction, "domain_events", "payload_version")?
    } else {
        false
    };
    let schema_version = if had_schema_version_table {
        let maybe_version: Option<i64> = transaction
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .map_err(to_port_error)?;
        match maybe_version {
            Some(v) => Some(v),
            None => {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema_version table is empty".to_string(),
                });
            }
        }
    } else {
        None
    };

    transaction
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS schema_version (
                 version INTEGER NOT NULL PRIMARY KEY,
                 applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS artifacts (
                 id INTEGER NOT NULL PRIMARY KEY,
                 title TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS artifact_chunks (
                 artifact_id INTEGER NOT NULL,
                 related_id INTEGER NOT NULL,
                 PRIMARY KEY (artifact_id, related_id),
                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS artifact_cards (
                 artifact_id INTEGER NOT NULL,
                 related_id INTEGER NOT NULL,
                 PRIMARY KEY (artifact_id, related_id),
                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS artifact_claims (
                 artifact_id INTEGER NOT NULL,
                 related_id INTEGER NOT NULL,
                 PRIMARY KEY (artifact_id, related_id),
                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS artifact_evidences (
                 artifact_id INTEGER NOT NULL,
                 related_id INTEGER NOT NULL,
                 PRIMARY KEY (artifact_id, related_id),
                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS chunks (
                 id INTEGER NOT NULL PRIMARY KEY,
                 artifact_id INTEGER NOT NULL,
                 chunk_order INTEGER NOT NULL,
                 text TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_chunks_artifact_order
                 ON chunks(artifact_id, chunk_order, id);
             CREATE TABLE IF NOT EXISTS cards (
                 id INTEGER NOT NULL PRIMARY KEY,
                 artifact_id INTEGER NOT NULL,
                 title TEXT NOT NULL,
                 body TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_cards_artifact
                 ON cards(artifact_id, id);
             CREATE TABLE IF NOT EXISTS card_claims (
                 card_id INTEGER NOT NULL,
                 claim_id INTEGER NOT NULL,
                 PRIMARY KEY (card_id, claim_id),
                 FOREIGN KEY (card_id) REFERENCES cards(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS evidence (
                 id INTEGER NOT NULL PRIMARY KEY,
                 artifact_id INTEGER NOT NULL,
                 claim_id INTEGER,
                 kind_json TEXT NOT NULL,
                 excerpt TEXT NOT NULL,
                 observed_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_evidence_artifact
                 ON evidence(artifact_id, id);
             CREATE TABLE IF NOT EXISTS domain_events (
                 id INTEGER NOT NULL PRIMARY KEY,
                 sequence INTEGER NOT NULL UNIQUE,
                 event_kind TEXT NOT NULL,
                 artifact_id INTEGER,
                 payload_json TEXT NOT NULL,
                 payload_version INTEGER NOT NULL DEFAULT 2
             );
             CREATE INDEX IF NOT EXISTS idx_domain_events_artifact_sequence
                 ON domain_events(artifact_id, sequence);",
        )
        .map_err(to_port_error)?;

    match schema_version {
        Some(1) => {
            if !had_domain_events_table {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema: domain_events table missing".to_string(),
                });
            }

            if had_payload_version_column {
                validate_columns(&transaction, &DOMAIN_EVENTS_V2_COLUMNS)?;
            } else {
                validate_columns(&transaction, &DOMAIN_EVENTS_V1_COLUMNS)?;
                transaction
                    .execute_batch(
                        "ALTER TABLE domain_events ADD COLUMN payload_version INTEGER NOT NULL DEFAULT 1;",
                    )
                    .map_err(to_port_error)?;
            }

            transaction
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
                    [CURRENT_SCHEMA_VERSION],
                )
                .map_err(to_port_error)?;
        }
        Some(2) => {
            if !had_domain_events_table {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema: domain_events table missing".to_string(),
                });
            }
            if had_payload_version_column {
                validate_columns(&transaction, &DOMAIN_EVENTS_V2_COLUMNS)?;
            } else {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema: missing payload_version column".to_string(),
                });
            }
        }
        Some(version) => {
            return Err(PortError::Internal {
                message: format!(
                    "unsupported sqlite schema version {version}; expected {CURRENT_SCHEMA_VERSION}"
                ),
            });
        }
        None => {
            if had_domain_events_table {
                if had_payload_version_column {
                    return Err(PortError::Internal {
                        message:
                            "malformed sqlite schema: schema_version table missing for domain_events v2 schema"
                                .to_string(),
                    });
                }
                validate_columns(&transaction, &DOMAIN_EVENTS_V1_COLUMNS)?;
                transaction
                    .execute_batch(
                        "ALTER TABLE domain_events ADD COLUMN payload_version INTEGER NOT NULL DEFAULT 1;",
                    )
                    .map_err(to_port_error)?;
            } else {
                validate_columns(&transaction, &DOMAIN_EVENTS_V2_COLUMNS)?;
            }

            transaction
                .execute(
                    "INSERT INTO schema_version (version) VALUES (?1)",
                    [CURRENT_SCHEMA_VERSION],
                )
                .map_err(to_port_error)?;
        }
    }

    validate_domain_events_schema(&transaction)?;
    validate_event_order(&transaction)?;
    validate_stored_event_payloads(&transaction)?;
    transaction.commit().map_err(to_port_error)
}

const DOMAIN_EVENTS_V1_COLUMNS: [&str; 5] = [
    "id",
    "sequence",
    "event_kind",
    "artifact_id",
    "payload_json",
];
const DOMAIN_EVENTS_V2_COLUMNS: [&str; 6] = [
    "id",
    "sequence",
    "event_kind",
    "artifact_id",
    "payload_json",
    "payload_version",
];

fn table_exists(connection: &Connection, table: &str) -> Result<bool, PortError> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?1",
            params![table],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    Ok(exists.is_some())
}

fn table_has_column(connection: &Connection, table: &str, column: &str) -> Result<bool, PortError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let name: String = row.get(1).map_err(to_port_error)?;
        if name == column {
            return Ok(true);
        }
    }

    Ok(false)
}

fn validate_columns(connection: &Connection, required: &[&str]) -> Result<(), PortError> {
    for column in required {
        if !table_has_column(connection, "domain_events", column)? {
            return Err(PortError::Internal {
                message: format!("malformed domain_events table missing required column {column}"),
            });
        }
    }

    Ok(())
}

fn validate_domain_events_schema(connection: &Connection) -> Result<(), PortError> {
    let mut statement = connection
        .prepare("PRAGMA table_info(domain_events)")
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        columns.push((
            row.get::<_, String>(1).map_err(to_port_error)?,
            row.get::<_, String>(2).map_err(to_port_error)?,
            row.get::<_, i64>(3).map_err(to_port_error)?,
            row.get::<_, i64>(5).map_err(to_port_error)?,
        ));
    }

    for (name, expected_type, require_not_null, require_nullable, require_primary_key) in [
        ("id", "INTEGER", true, false, true),
        ("sequence", "INTEGER", true, false, false),
        ("event_kind", "TEXT", true, false, false),
        ("artifact_id", "INTEGER", false, true, false),
        ("payload_json", "TEXT", true, false, false),
        ("payload_version", "INTEGER", true, false, false),
    ] {
        let Some((_, actual_type, not_null, primary_key)) =
            columns.iter().find(|(column, _, _, _)| column == name)
        else {
            return Err(PortError::Internal {
                message: format!("malformed domain_events table missing required column {name}"),
            });
        };
        if !actual_type.trim().eq_ignore_ascii_case(expected_type)
            || (require_not_null && *not_null == 0)
            || (require_nullable && *not_null != 0)
            || (require_primary_key && *primary_key == 0)
        {
            return Err(PortError::Internal {
                message: format!("malformed domain_events column {name}"),
            });
        }
    }

    let indexes = {
        let mut statement = connection
            .prepare("PRAGMA index_list(domain_events)")
            .map_err(to_port_error)?;
        let mut rows = statement.query([]).map_err(to_port_error)?;
        let mut indexes = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            indexes.push((
                row.get::<_, String>(1).map_err(to_port_error)?,
                row.get::<_, i64>(2).map_err(to_port_error)? != 0,
            ));
        }
        indexes
    };
    let mut has_unique_sequence = false;
    let mut has_artifact_sequence_index = false;
    for (index_name, unique) in indexes {
        let quoted_name = index_name.replace('"', "\"\"");
        let mut statement = connection
            .prepare(&format!("PRAGMA index_info(\"{quoted_name}\")"))
            .map_err(to_port_error)?;
        let mut rows = statement.query([]).map_err(to_port_error)?;
        let mut index_columns = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            index_columns.push(row.get::<_, String>(2).map_err(to_port_error)?);
        }
        if unique && index_columns == ["sequence"] {
            has_unique_sequence = true;
        }
        if index_name == "idx_domain_events_artifact_sequence"
            && index_columns == ["artifact_id", "sequence"]
        {
            has_artifact_sequence_index = true;
        }
    }
    if !has_unique_sequence {
        return Err(PortError::Internal {
            message: "malformed domain_events table sequence is not unique".to_string(),
        });
    }
    if !has_artifact_sequence_index {
        return Err(PortError::Internal {
            message: "malformed domain_events artifact index".to_string(),
        });
    }
    Ok(())
}

fn validate_event_order(connection: &Connection) -> Result<(), PortError> {
    let mut statement = connection
        .prepare("SELECT id, sequence FROM domain_events ORDER BY sequence ASC")
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let mut expected = 1_u64;
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let id = i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?;
        let sequence = i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?;
        if id != expected || sequence != expected {
            return Err(PortError::Internal {
                message: format!(
                    "domain event log is not contiguous at expected {expected}: id {id}, sequence {sequence}"
                ),
            });
        }
        expected = expected.checked_add(1).ok_or_else(|| PortError::Internal {
            message: "domain event sequence exhausted u64 range".to_string(),
        })?;
    }
    Ok(())
}

fn validate_stored_event_payloads(connection: &Connection) -> Result<(), PortError> {
    let mut statement = connection
        .prepare(
            "SELECT event_kind, artifact_id, payload_json, payload_version
             FROM domain_events
             ORDER BY sequence ASC",
        )
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let stored_kind: String = row.get(0).map_err(to_port_error)?;
        let stored_artifact_id: Option<i64> = row.get(1).map_err(to_port_error)?;
        let payload_json: String = row.get(2).map_err(to_port_error)?;
        let payload_version: i64 = row.get(3).map_err(to_port_error)?;
        let payload = match payload_version {
            1 => {
                let legacy: LegacyStoredEventPayload =
                    serde_json::from_str(&payload_json).map_err(json_error)?;
                legacy.into_v2()?
            }
            2 => serde_json::from_str(&payload_json).map_err(json_error)?,
            version => {
                return Err(PortError::Internal {
                    message: format!("unsupported event payload version {version}"),
                });
            }
        };
        if stored_kind != payload.kind() {
            return Err(PortError::Internal {
                message: format!(
                    "event kind column {stored_kind} does not match payload kind {}",
                    payload.kind()
                ),
            });
        }
        let stored_artifact_id = stored_artifact_id.map(i64_to_u64).transpose()?;
        if stored_artifact_id != payload.filter_artifact_id() {
            return Err(PortError::Internal {
                message: "event artifact_id column does not match payload".to_string(),
            });
        }
    }
    Ok(())
}

fn load_id_set<T>(
    connection: &Connection,
    table: &str,
    artifact_id: ArtifactId,
    make: fn(u64) -> T,
) -> Result<BTreeSet<T>, PortError>
where
    T: Ord,
{
    let mut statement = connection
        .prepare(&format!(
            "SELECT related_id FROM {table} WHERE artifact_id = ?1 ORDER BY related_id"
        ))
        .map_err(to_port_error)?;
    let mut rows = statement
        .query(params![u64_to_i64(artifact_id.value())?])
        .map_err(to_port_error)?;
    let mut ids = BTreeSet::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let value = i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?;
        ids.insert(make(value));
    }
    Ok(ids)
}

fn replace_id_set(
    transaction: &Transaction<'_>,
    table: &str,
    artifact_id: ArtifactId,
    ids: impl Iterator<Item = u64>,
) -> Result<(), PortError> {
    transaction
        .execute(
            &format!("DELETE FROM {table} WHERE artifact_id = ?1"),
            params![u64_to_i64(artifact_id.value())?],
        )
        .map_err(to_port_error)?;

    for id in ids {
        transaction
            .execute(
                &format!("INSERT INTO {table} (artifact_id, related_id) VALUES (?1, ?2)"),
                params![u64_to_i64(artifact_id.value())?, u64_to_i64(id)?],
            )
            .map_err(to_port_error)?;
    }

    Ok(())
}

fn read_chunk(row: &rusqlite::Row<'_>) -> Result<Chunk, PortError> {
    let order = i64_to_u32(row.get::<_, i64>(2).map_err(to_port_error)?)?;
    Ok(Chunk {
        id: ChunkId::new(i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?),
        artifact_id: ArtifactId::new(i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?),
        order,
        text: row.get::<_, String>(3).map_err(to_port_error)?,
    })
}

fn read_card(row: &rusqlite::Row<'_>, connection: &Connection) -> Result<Card, PortError> {
    let id = CardId::new(i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?);
    Ok(Card {
        id,
        artifact_id: ArtifactId::new(i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?),
        title: row.get::<_, String>(2).map_err(to_port_error)?,
        body: row.get::<_, String>(3).map_err(to_port_error)?,
        claim_ids: load_card_claims(connection, id)?,
    })
}

fn load_card_claims(
    connection: &Connection,
    card_id: CardId,
) -> Result<BTreeSet<ClaimId>, PortError> {
    let mut statement = connection
        .prepare("SELECT claim_id FROM card_claims WHERE card_id = ?1 ORDER BY claim_id")
        .map_err(to_port_error)?;
    let mut rows = statement
        .query(params![u64_to_i64(card_id.value())?])
        .map_err(to_port_error)?;
    let mut ids = BTreeSet::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        ids.insert(ClaimId::new(i64_to_u64(
            row.get::<_, i64>(0).map_err(to_port_error)?,
        )?));
    }
    Ok(ids)
}

fn replace_card_claims(
    transaction: &Transaction<'_>,
    card_id: CardId,
    ids: impl Iterator<Item = u64>,
) -> Result<(), PortError> {
    transaction
        .execute(
            "DELETE FROM card_claims WHERE card_id = ?1",
            params![u64_to_i64(card_id.value())?],
        )
        .map_err(to_port_error)?;

    for id in ids {
        transaction
            .execute(
                "INSERT INTO card_claims (card_id, claim_id) VALUES (?1, ?2)",
                params![u64_to_i64(card_id.value())?, u64_to_i64(id)?],
            )
            .map_err(to_port_error)?;
    }

    Ok(())
}

fn read_evidence(row: &rusqlite::Row<'_>) -> Result<Evidence, PortError> {
    let kind_json = row.get::<_, String>(3).map_err(to_port_error)?;
    let kind = serde_json::from_str::<StoredEvidenceKind>(&kind_json)
        .map_err(json_error)?
        .into_domain();
    Ok(Evidence {
        id: EvidenceId::new(i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?),
        artifact_id: ArtifactId::new(i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?),
        claim_id: optional_i64_to_u64(row.get::<_, Option<i64>>(2).map_err(to_port_error)?)?
            .map(ClaimId::new),
        kind,
        excerpt: row.get::<_, String>(4).map_err(to_port_error)?,
        observed_at: LogicalTick::new(i64_to_u64(row.get::<_, i64>(5).map_err(to_port_error)?)?),
    })
}

#[derive(Debug)]
struct StoredEvent {
    id: u64,
    sequence: u64,
    kind: &'static str,
    artifact_id: Option<u64>,
    payload_json: String,
    payload_version: i64,
}

impl StoredEvent {
    fn from_domain(envelope: &DomainEventEnvelope) -> Result<Self, PortError> {
        let payload = StoredEventPayload::from_domain(&envelope.event);
        Ok(Self {
            id: envelope.id.value(),
            sequence: envelope.sequence.value(),
            kind: payload.kind(),
            artifact_id: payload.filter_artifact_id(),
            payload_json: serde_json::to_string(&payload).map_err(json_error)?,
            payload_version: 2,
        })
    }

    fn into_domain(self) -> Result<DomainEventEnvelope, PortError> {
        let payload = match self.payload_version {
            1 => {
                let legacy: LegacyStoredEventPayload =
                    serde_json::from_str(&self.payload_json).map_err(json_error)?;
                legacy.into_v2()?
            }
            2 => serde_json::from_str(&self.payload_json).map_err(json_error)?,
            other => {
                return Err(PortError::Internal {
                    message: format!("unsupported payload version {}", other),
                });
            }
        };
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event_kind", rename_all = "snake_case", deny_unknown_fields)]
enum LegacyStoredEventPayload {
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
        task_id: u64,
        approved: bool,
    },
    TickObserved {
        at: u64,
    },
}
impl LegacyStoredEventPayload {
    fn into_v2(self) -> Result<StoredEventPayload, PortError> {
        let unsupported = |kind: &str, field: &str| PortError::InvalidInput {
            message: format!("V1 {kind} event is missing required field(s): {field}"),
        };
        match self {
            Self::ArtifactRegistered { artifact_id, title } => {
                Ok(StoredEventPayload::ArtifactRegistered { artifact_id, title })
            }
            Self::ChunkRegistered { .. } => Err(unsupported("ChunkRegistered", "text")),
            Self::CardCreated { .. } => Err(unsupported("CardCreated", "title and body")),
            Self::ClaimCreated { .. } => Err(unsupported("ClaimCreated", "text and evidence_ids")),
            Self::EvidenceRecorded { .. } => {
                Err(unsupported("EvidenceRecorded", "excerpt and observed_at"))
            }
            Self::TaskOpened { .. } => Err(unsupported("TaskOpened", "artifact_id")),
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
            Self::RelationCreated { .. } => Err(unsupported(
                "RelationCreated",
                "source, kind, target, evidence_id, confidence_milli",
            )),
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
            Self::ApprovalRecorded { task_id, approved } => {
                Ok(StoredEventPayload::ApprovalRecorded { task_id, approved })
            }
            Self::TickObserved { at } => Ok(StoredEventPayload::TickObserved { at }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event_kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredEventPayload {
    ArtifactRegistered {
        artifact_id: u64,
        title: String,
    },
    ChunkRegistered {
        chunk_id: u64,
        artifact_id: u64,
        order: u32,
        text: String,
    },
    CardCreated {
        card_id: u64,
        artifact_id: u64,
        title: String,
        body: String,
    },
    ClaimCreated {
        claim_id: u64,
        artifact_id: u64,
        text: String,
        evidence_ids: Vec<u64>,
    },
    EvidenceRecorded {
        evidence_id: u64,
        artifact_id: u64,
        claim_id: Option<u64>,
        evidence_kind: StoredEvidenceKind,
        excerpt: String,
        observed_at: u64,
    },
    TaskOpened {
        task_id: u64,
        title: String,
        priority: StoredTaskPriority,
        artifact_id: Option<u64>,
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
        source: StoredRelationEndpoint,
        kind: StoredRelationKind,
        target: StoredRelationEndpoint,
        evidence_id: Option<u64>,
        confidence_milli: u16,
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
        task_id: u64,
        approved: bool,
    },
    TickObserved {
        at: u64,
    },
}

impl StoredEventPayload {
    fn from_domain(event: &DomainEvent) -> Self {
        match event {
            DomainEvent::ArtifactRegistered { artifact_id, title } => Self::ArtifactRegistered {
                artifact_id: artifact_id.value(),
                title: title.clone(),
            },
            DomainEvent::ChunkRegistered {
                chunk_id,
                artifact_id,
                order,
                text,
            } => Self::ChunkRegistered {
                chunk_id: chunk_id.value(),
                artifact_id: artifact_id.value(),
                order: *order,
                text: text.clone(),
            },
            DomainEvent::CardCreated {
                card_id,
                artifact_id,
                title,
                body,
            } => Self::CardCreated {
                card_id: card_id.value(),
                artifact_id: artifact_id.value(),
                title: title.clone(),
                body: body.clone(),
            },
            DomainEvent::ClaimCreated {
                claim_id,
                artifact_id,
                text,
                evidence_ids,
            } => Self::ClaimCreated {
                claim_id: claim_id.value(),
                artifact_id: artifact_id.value(),
                text: text.clone(),
                evidence_ids: evidence_ids.iter().map(|id| id.value()).collect(),
            },
            DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                kind,
                excerpt,
                observed_at,
            } => Self::EvidenceRecorded {
                evidence_id: evidence_id.value(),
                artifact_id: artifact_id.value(),
                claim_id: claim_id.map(|id| id.value()),
                evidence_kind: StoredEvidenceKind::from_domain(kind),
                excerpt: excerpt.clone(),
                observed_at: observed_at.value(),
            },
            DomainEvent::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => Self::TaskOpened {
                task_id: task_id.value(),
                title: title.clone(),
                priority: StoredTaskPriority::from_domain(*priority),
                artifact_id: artifact_id.map(|id| id.value()),
            },
            DomainEvent::TaskStatusChanged { task_id, from, to } => Self::TaskStatusChanged {
                task_id: task_id.value(),
                from: StoredTaskStatus::from_domain(*from),
                to: StoredTaskStatus::from_domain(*to),
            },
            DomainEvent::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => Self::TaskCompletionRecorded {
                task_id: task_id.value(),
                status: StoredTaskStatus::from_domain(*status),
                validation_report_id: validation_report_id.value(),
            },
            DomainEvent::ClaimValidationUpdated { claim_id, status } => {
                Self::ClaimValidationUpdated {
                    claim_id: claim_id.value(),
                    status: StoredClaimStatus::from_domain(status),
                }
            }
            DomainEvent::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => Self::ClaimEvidenceLinked {
                claim_id: claim_id.value(),
                evidence_id: evidence_id.value(),
            },
            DomainEvent::RelationCreated {
                relation_id,
                source,
                kind,
                target,
                evidence_id,
                confidence_milli,
            } => Self::RelationCreated {
                relation_id: relation_id.value(),
                source: StoredRelationEndpoint::from_domain(source),
                kind: StoredRelationKind::from_domain(kind),
                target: StoredRelationEndpoint::from_domain(target),
                evidence_id: evidence_id.map(|id| id.value()),
                confidence_milli: *confidence_milli,
            },
            DomainEvent::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => Self::MemoryCandidateCreated {
                candidate_id: candidate_id.value(),
                claim_id: claim_id.value(),
                evidence_ids: evidence_ids
                    .iter()
                    .map(|evidence_id| evidence_id.value())
                    .collect(),
                confidence_milli: *confidence_milli,
            },
            DomainEvent::MemoryPromoted {
                memory_id,
                candidate_id,
            } => Self::MemoryPromoted {
                memory_id: memory_id.value(),
                candidate_id: candidate_id.value(),
            },
            DomainEvent::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => Self::MemoryContradicted {
                memory_id: memory_id.value(),
                contradicting_candidate_id: contradicting_candidate_id.value(),
            },
            DomainEvent::MemoryDeprecated { memory_id } => Self::MemoryDeprecated {
                memory_id: memory_id.value(),
            },
            DomainEvent::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => Self::MemorySuperseded {
                memory_id: memory_id.value(),
                by_memory_id: by_memory_id.value(),
            },
            DomainEvent::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => Self::ValidationReportCreated {
                report_id: report_id.value(),
                task_id: task_id.map(|task_id| task_id.value()),
                passed: *passed,
                warnings: warnings.clone(),
            },
            DomainEvent::UserIntentObserved { task_id, title } => Self::UserIntentObserved {
                task_id: task_id.value(),
                title: title.clone(),
            },
            DomainEvent::ArtifactParsed {
                artifact_id,
                chunks_added,
            } => Self::ArtifactParsed {
                artifact_id: artifact_id.value(),
                chunks_added: *chunks_added,
            },
            DomainEvent::SearchCompleted {
                artifact_id,
                cards_added,
            } => Self::SearchCompleted {
                artifact_id: artifact_id.value(),
                cards_added: *cards_added,
            },
            DomainEvent::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            } => Self::HarnessRunCompleted {
                task_id: task_id.map(|id| id.value()),
                command: command.clone(),
                exit_code: *exit_code,
            },
            DomainEvent::ApprovalRecorded { task_id, approved } => Self::ApprovalRecorded {
                task_id: task_id.value(),
                approved: *approved,
            },
            DomainEvent::TickObserved { at } => Self::TickObserved { at: at.value() },
        }
    }

    fn into_domain(self) -> DomainEvent {
        match self {
            Self::ArtifactRegistered { artifact_id, title } => DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(artifact_id),
                title,
            },
            Self::ChunkRegistered {
                chunk_id,
                artifact_id,
                order,
                text,
            } => DomainEvent::ChunkRegistered {
                chunk_id: ChunkId::new(chunk_id),
                artifact_id: ArtifactId::new(artifact_id),
                order,
                text,
            },
            Self::CardCreated {
                card_id,
                artifact_id,
                title,
                body,
            } => DomainEvent::CardCreated {
                card_id: CardId::new(card_id),
                artifact_id: ArtifactId::new(artifact_id),
                title,
                body,
            },
            Self::ClaimCreated {
                claim_id,
                artifact_id,
                text,
                evidence_ids,
            } => DomainEvent::ClaimCreated {
                claim_id: ClaimId::new(claim_id),
                artifact_id: ArtifactId::new(artifact_id),
                text,
                evidence_ids: evidence_ids.into_iter().map(EvidenceId::new).collect(),
            },
            Self::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                evidence_kind,
                excerpt,
                observed_at,
            } => DomainEvent::EvidenceRecorded {
                evidence_id: EvidenceId::new(evidence_id),
                artifact_id: ArtifactId::new(artifact_id),
                claim_id: claim_id.map(ClaimId::new),
                kind: evidence_kind.into_domain(),
                excerpt,
                observed_at: maestria_domain::LogicalTick::new(observed_at),
            },
            Self::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => DomainEvent::TaskOpened {
                task_id: TaskId::new(task_id),
                title,
                priority: priority.into_domain(),
                artifact_id: artifact_id.map(ArtifactId::new),
            },
            Self::TaskStatusChanged { task_id, from, to } => DomainEvent::TaskStatusChanged {
                task_id: TaskId::new(task_id),
                from: from.into_domain(),
                to: to.into_domain(),
            },
            Self::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => DomainEvent::TaskCompletionRecorded {
                task_id: TaskId::new(task_id),
                status: status.into_domain(),
                validation_report_id: ValidationReportId::new(validation_report_id),
            },
            Self::ClaimValidationUpdated { claim_id, status } => {
                DomainEvent::ClaimValidationUpdated {
                    claim_id: ClaimId::new(claim_id),
                    status: status.into_domain(),
                }
            }
            Self::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => DomainEvent::ClaimEvidenceLinked {
                claim_id: ClaimId::new(claim_id),
                evidence_id: EvidenceId::new(evidence_id),
            },
            Self::RelationCreated {
                relation_id,
                source,
                kind,
                target,
                evidence_id,
                confidence_milli,
            } => DomainEvent::RelationCreated {
                relation_id: maestria_domain::RelationId::new(relation_id),
                source: source.into_domain(),
                kind: kind.into_domain(),
                target: target.into_domain(),
                evidence_id: evidence_id.map(maestria_domain::EvidenceId::new),
                confidence_milli,
            },
            Self::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => DomainEvent::MemoryCandidateCreated {
                candidate_id: maestria_domain::MemoryCandidateId::new(candidate_id),
                claim_id: ClaimId::new(claim_id),
                evidence_ids: evidence_ids.into_iter().map(EvidenceId::new).collect(),
                confidence_milli,
            },
            Self::MemoryPromoted {
                memory_id,
                candidate_id,
            } => DomainEvent::MemoryPromoted {
                memory_id: maestria_domain::MemoryId::new(memory_id),
                candidate_id: maestria_domain::MemoryCandidateId::new(candidate_id),
            },
            Self::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => DomainEvent::MemoryContradicted {
                memory_id: maestria_domain::MemoryId::new(memory_id),
                contradicting_candidate_id: maestria_domain::MemoryCandidateId::new(
                    contradicting_candidate_id,
                ),
            },
            Self::MemoryDeprecated { memory_id } => DomainEvent::MemoryDeprecated {
                memory_id: maestria_domain::MemoryId::new(memory_id),
            },
            Self::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => DomainEvent::MemorySuperseded {
                memory_id: maestria_domain::MemoryId::new(memory_id),
                by_memory_id: maestria_domain::MemoryId::new(by_memory_id),
            },
            Self::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => DomainEvent::ValidationReportCreated {
                report_id: ValidationReportId::new(report_id),
                task_id: task_id.map(TaskId::new),
                passed,
                warnings,
            },
            Self::UserIntentObserved { task_id, title } => DomainEvent::UserIntentObserved {
                task_id: TaskId::new(task_id),
                title,
            },
            Self::ArtifactParsed {
                artifact_id,
                chunks_added,
            } => DomainEvent::ArtifactParsed {
                artifact_id: ArtifactId::new(artifact_id),
                chunks_added,
            },
            Self::SearchCompleted {
                artifact_id,
                cards_added,
            } => DomainEvent::SearchCompleted {
                artifact_id: ArtifactId::new(artifact_id),
                cards_added,
            },
            Self::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            } => DomainEvent::HarnessRunCompleted {
                task_id: task_id.map(TaskId::new),
                command,
                exit_code,
            },
            Self::ApprovalRecorded { task_id, approved } => DomainEvent::ApprovalRecorded {
                task_id: TaskId::new(task_id),
                approved,
            },
            Self::TickObserved { at } => DomainEvent::TickObserved {
                at: LogicalTick::new(at),
            },
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::ArtifactRegistered { .. } => "artifact_registered",
            Self::ChunkRegistered { .. } => "chunk_registered",
            Self::CardCreated { .. } => "card_created",
            Self::ClaimCreated { .. } => "claim_created",
            Self::EvidenceRecorded { .. } => "evidence_recorded",
            Self::TaskOpened { .. } => "task_opened",
            Self::TaskStatusChanged { .. } => "task_status_changed",
            Self::TaskCompletionRecorded { .. } => "task_completion_recorded",
            Self::ClaimValidationUpdated { .. } => "claim_validation_updated",
            Self::ClaimEvidenceLinked { .. } => "claim_evidence_linked",
            Self::RelationCreated { .. } => "relation_created",
            Self::MemoryCandidateCreated { .. } => "memory_candidate_created",
            Self::MemoryPromoted { .. } => "memory_promoted",
            Self::MemoryContradicted { .. } => "memory_contradicted",
            Self::MemoryDeprecated { .. } => "memory_deprecated",
            Self::MemorySuperseded { .. } => "memory_superseded",
            Self::ValidationReportCreated { .. } => "validation_report_created",
            Self::UserIntentObserved { .. } => "user_intent_observed",
            Self::ArtifactParsed { .. } => "artifact_parsed",
            Self::SearchCompleted { .. } => "search_completed",
            Self::HarnessRunCompleted { .. } => "harness_run_completed",
            Self::ApprovalRecorded { .. } => "approval_recorded",
            Self::TickObserved { .. } => "tick_observed",
        }
    }

    fn filter_artifact_id(&self) -> Option<u64> {
        match self {
            Self::ArtifactRegistered { artifact_id, .. }
            | Self::ChunkRegistered { artifact_id, .. }
            | Self::CardCreated { artifact_id, .. }
            | Self::ClaimCreated { artifact_id, .. }
            | Self::EvidenceRecorded { artifact_id, .. }
            | Self::ArtifactParsed { artifact_id, .. }
            | Self::SearchCompleted { artifact_id, .. } => Some(*artifact_id),
            Self::TaskOpened {
                artifact_id: Some(artifact_id),
                ..
            } => Some(*artifact_id),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredRelationEndpoint {
    Artifact { artifact_id: u64 },
    Claim { claim_id: u64 },
    Task { task_id: u64 },
    Memory { memory_id: u64 },
    Card { card_id: u64 },
}

impl StoredRelationEndpoint {
    fn from_domain(endpoint: &maestria_domain::RelationEndpoint) -> Self {
        match endpoint {
            maestria_domain::RelationEndpoint::Artifact(id) => Self::Artifact {
                artifact_id: id.value(),
            },
            maestria_domain::RelationEndpoint::Claim(id) => Self::Claim {
                claim_id: id.value(),
            },
            maestria_domain::RelationEndpoint::Task(id) => Self::Task {
                task_id: id.value(),
            },
            maestria_domain::RelationEndpoint::Memory(id) => Self::Memory {
                memory_id: id.value(),
            },
            maestria_domain::RelationEndpoint::Card(id) => Self::Card {
                card_id: id.value(),
            },
        }
    }

    fn into_domain(self) -> maestria_domain::RelationEndpoint {
        match self {
            Self::Artifact { artifact_id } => maestria_domain::RelationEndpoint::Artifact(
                maestria_domain::ArtifactId::new(artifact_id),
            ),
            Self::Claim { claim_id } => {
                maestria_domain::RelationEndpoint::Claim(maestria_domain::ClaimId::new(claim_id))
            }
            Self::Task { task_id } => {
                maestria_domain::RelationEndpoint::Task(maestria_domain::TaskId::new(task_id))
            }
            Self::Memory { memory_id } => {
                maestria_domain::RelationEndpoint::Memory(maestria_domain::MemoryId::new(memory_id))
            }
            Self::Card { card_id } => {
                maestria_domain::RelationEndpoint::Card(maestria_domain::CardId::new(card_id))
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredRelationKind {
    Contains,
    Defines,
    Supports,
    Contradicts,
    UsedEvidence,
    BasedOn,
    DerivedFrom,
    AppliesTo,
    RelatedTo,
}

impl StoredRelationKind {
    fn from_domain(kind: &maestria_domain::RelationKind) -> Self {
        match kind {
            maestria_domain::RelationKind::Contains => Self::Contains,
            maestria_domain::RelationKind::Defines => Self::Defines,
            maestria_domain::RelationKind::Supports => Self::Supports,
            maestria_domain::RelationKind::Contradicts => Self::Contradicts,
            maestria_domain::RelationKind::UsedEvidence => Self::UsedEvidence,
            maestria_domain::RelationKind::BasedOn => Self::BasedOn,
            maestria_domain::RelationKind::DerivedFrom => Self::DerivedFrom,
            maestria_domain::RelationKind::AppliesTo => Self::AppliesTo,
            maestria_domain::RelationKind::RelatedTo => Self::RelatedTo,
        }
    }

    fn into_domain(self) -> maestria_domain::RelationKind {
        match self {
            Self::Contains => maestria_domain::RelationKind::Contains,
            Self::Defines => maestria_domain::RelationKind::Defines,
            Self::Supports => maestria_domain::RelationKind::Supports,
            Self::Contradicts => maestria_domain::RelationKind::Contradicts,
            Self::UsedEvidence => maestria_domain::RelationKind::UsedEvidence,
            Self::BasedOn => maestria_domain::RelationKind::BasedOn,
            Self::DerivedFrom => maestria_domain::RelationKind::DerivedFrom,
            Self::AppliesTo => maestria_domain::RelationKind::AppliesTo,
            Self::RelatedTo => maestria_domain::RelationKind::RelatedTo,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredEvidenceKind {
    FileSpan {
        path: String,
        start: usize,
        end: usize,
        content_hash: String,
    },
    PdfSpan {
        blob: u64,
        page_start: u32,
        page_end: u32,
    },
    WebSnapshot {
        url: String,
        snapshot: u64,
        fetched_at: u64,
        content_hash: String,
    },
    CommandOutput {
        harness_run: u64,
        stream: StoredOutputStream,
        blob: u64,
    },
    TestResult {
        harness_run: u64,
        status: StoredTestStatus,
        log: u64,
    },
    Diff {
        harness_run: u64,
        patch_blob: u64,
    },
    Validation {
        report_id: u64,
    },
}

impl StoredEvidenceKind {
    fn from_domain(kind: &EvidenceKind) -> Self {
        match kind {
            EvidenceKind::FileSpan {
                path,
                range,
                content_hash,
            } => Self::FileSpan {
                path: path.clone(),
                start: range.start,
                end: range.end,
                content_hash: content_hash.clone(),
            },
            EvidenceKind::PdfSpan {
                blob,
                page_start,
                page_end,
            } => Self::PdfSpan {
                blob: blob.value(),
                page_start: *page_start,
                page_end: *page_end,
            },
            EvidenceKind::WebSnapshot {
                url,
                snapshot,
                fetched_at,
                content_hash,
            } => Self::WebSnapshot {
                url: url.clone(),
                snapshot: snapshot.value(),
                fetched_at: fetched_at.value(),
                content_hash: content_hash.clone(),
            },
            EvidenceKind::CommandOutput {
                harness_run,
                stream,
                blob,
            } => Self::CommandOutput {
                harness_run: harness_run.value(),
                stream: StoredOutputStream::from_domain(*stream),
                blob: blob.value(),
            },
            EvidenceKind::TestResult {
                harness_run,
                status,
                log,
            } => Self::TestResult {
                harness_run: harness_run.value(),
                status: StoredTestStatus::from_domain(*status),
                log: log.value(),
            },
            EvidenceKind::Diff {
                harness_run,
                patch_blob,
            } => Self::Diff {
                harness_run: harness_run.value(),
                patch_blob: patch_blob.value(),
            },
            EvidenceKind::Validation { report_id } => Self::Validation {
                report_id: report_id.value(),
            },
        }
    }

    fn into_domain(self) -> EvidenceKind {
        match self {
            Self::FileSpan {
                path,
                start,
                end,
                content_hash,
            } => EvidenceKind::FileSpan {
                path,
                range: ContentRange { start, end },
                content_hash,
            },
            Self::PdfSpan {
                blob,
                page_start,
                page_end,
            } => EvidenceKind::PdfSpan {
                blob: BlobId::new(blob),
                page_start,
                page_end,
            },
            Self::WebSnapshot {
                url,
                snapshot,
                fetched_at,
                content_hash,
            } => EvidenceKind::WebSnapshot {
                url,
                snapshot: BlobId::new(snapshot),
                fetched_at: LogicalTick::new(fetched_at),
                content_hash,
            },
            Self::CommandOutput {
                harness_run,
                stream,
                blob,
            } => EvidenceKind::CommandOutput {
                harness_run: HarnessRunId::new(harness_run),
                stream: stream.into_domain(),
                blob: BlobId::new(blob),
            },
            Self::TestResult {
                harness_run,
                status,
                log,
            } => EvidenceKind::TestResult {
                harness_run: HarnessRunId::new(harness_run),
                status: status.into_domain(),
                log: BlobId::new(log),
            },
            Self::Diff {
                harness_run,
                patch_blob,
            } => EvidenceKind::Diff {
                harness_run: HarnessRunId::new(harness_run),
                patch_blob: BlobId::new(patch_blob),
            },
            Self::Validation { report_id } => EvidenceKind::Validation {
                report_id: ValidationReportId::new(report_id),
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredOutputStream {
    Stdout,
    Stderr,
    Combined,
}

impl StoredOutputStream {
    fn from_domain(stream: OutputStream) -> Self {
        match stream {
            OutputStream::Stdout => Self::Stdout,
            OutputStream::Stderr => Self::Stderr,
            OutputStream::Combined => Self::Combined,
        }
    }

    fn into_domain(self) -> OutputStream {
        match self {
            Self::Stdout => OutputStream::Stdout,
            Self::Stderr => OutputStream::Stderr,
            Self::Combined => OutputStream::Combined,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredTestStatus {
    Passed,
    Failed,
    TimedOut,
}

impl StoredTestStatus {
    fn from_domain(status: maestria_domain::TestStatus) -> Self {
        match status {
            maestria_domain::TestStatus::Passed => Self::Passed,
            maestria_domain::TestStatus::Failed => Self::Failed,
            maestria_domain::TestStatus::TimedOut => Self::TimedOut,
        }
    }

    fn into_domain(self) -> maestria_domain::TestStatus {
        match self {
            Self::Passed => maestria_domain::TestStatus::Passed,
            Self::Failed => maestria_domain::TestStatus::Failed,
            Self::TimedOut => maestria_domain::TestStatus::TimedOut,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredTaskPriority {
    Low,
    Normal,
    High,
}

impl StoredTaskPriority {
    fn from_domain(priority: TaskPriority) -> Self {
        match priority {
            TaskPriority::Low => Self::Low,
            TaskPriority::Normal => Self::Normal,
            TaskPriority::High => Self::High,
        }
    }

    fn into_domain(self) -> TaskPriority {
        match self {
            Self::Low => TaskPriority::Low,
            Self::Normal => TaskPriority::Normal,
            Self::High => TaskPriority::High,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredTaskStatus {
    Draft,
    Open,
    Active,
    Validating,
    Blocked,
    CompletedVerified,
    CompletedWithWarnings,
    Failed,
    Cancelled,
}

impl StoredTaskStatus {
    fn from_domain(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Draft => Self::Draft,
            TaskStatus::Open => Self::Open,
            TaskStatus::Active => Self::Active,
            TaskStatus::Validating => Self::Validating,
            TaskStatus::Blocked => Self::Blocked,
            TaskStatus::CompletedVerified => Self::CompletedVerified,
            TaskStatus::CompletedWithWarnings => Self::CompletedWithWarnings,
            TaskStatus::Failed => Self::Failed,
            TaskStatus::Cancelled => Self::Cancelled,
        }
    }

    fn into_domain(self) -> TaskStatus {
        match self {
            Self::Draft => TaskStatus::Draft,
            Self::Open => TaskStatus::Open,
            Self::Active => TaskStatus::Active,
            Self::Validating => TaskStatus::Validating,
            Self::Blocked => TaskStatus::Blocked,
            Self::CompletedVerified => TaskStatus::CompletedVerified,
            Self::CompletedWithWarnings => TaskStatus::CompletedWithWarnings,
            Self::Failed => TaskStatus::Failed,
            Self::Cancelled => TaskStatus::Cancelled,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredClaimStatus {
    Draft,
    Proposed,
    Verified,
    Disputed,
    Archived,
}

impl StoredClaimStatus {
    fn from_domain(status: &ClaimStatus) -> Self {
        match status {
            ClaimStatus::Draft => Self::Draft,
            ClaimStatus::Proposed => Self::Proposed,
            ClaimStatus::Verified => Self::Verified,
            ClaimStatus::Disputed => Self::Disputed,
            ClaimStatus::Archived => Self::Archived,
        }
    }

    fn into_domain(self) -> ClaimStatus {
        match self {
            Self::Draft => ClaimStatus::Draft,
            Self::Proposed => ClaimStatus::Proposed,
            Self::Verified => ClaimStatus::Verified,
            Self::Disputed => ClaimStatus::Disputed,
            Self::Archived => ClaimStatus::Archived,
        }
    }
}

fn read_stored_event(row: &rusqlite::Row<'_>) -> Result<StoredEvent, PortError> {
    Ok(StoredEvent {
        id: i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?,
        sequence: i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?,
        kind: leaked_kind(row.get::<_, String>(2).map_err(to_port_error)?)?,
        artifact_id: optional_i64_to_u64(row.get::<_, Option<i64>>(3).map_err(to_port_error)?)?,
        payload_json: row.get::<_, String>(4).map_err(to_port_error)?,
        payload_version: row.get::<_, i64>(5).map_err(to_port_error)?,
    })
}

fn leaked_kind(kind: String) -> Result<&'static str, PortError> {
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
        other => Err(PortError::Internal {
            message: format!("unknown stored event kind {other}"),
        }),
    }
}

fn u64_to_i64(value: u64) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| PortError::InvalidInput {
        message: format!("identifier value {value} exceeds sqlite INTEGER range"),
    })
}

fn optional_u64_to_i64(value: Option<u64>) -> Result<Option<i64>, PortError> {
    value.map(u64_to_i64).transpose()
}

fn i64_to_u64(value: i64) -> Result<u64, PortError> {
    u64::try_from(value).map_err(|_| PortError::Internal {
        message: format!("stored negative identifier value {value}"),
    })
}

fn i64_to_u32(value: i64) -> Result<u32, PortError> {
    u32::try_from(value).map_err(|_| PortError::Internal {
        message: format!("stored chunk order value {value} is outside u32 range"),
    })
}

fn optional_i64_to_u64(value: Option<i64>) -> Result<Option<u64>, PortError> {
    value.map(i64_to_u64).transpose()
}

fn json_error(error: serde_json::Error) -> PortError {
    PortError::Internal {
        message: format!("event payload serialization failed: {error}"),
    }
}

fn to_port_error(error: rusqlite::Error) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
    }
}

fn map_append_error(error: rusqlite::Error) -> PortError {
    if let rusqlite::Error::SqliteFailure(failure, _) = &error
        && failure.code == ErrorCode::ConstraintViolation
    {
        return PortError::Conflict {
            message: "domain event id or sequence already exists".to_string(),
        };
    }
    to_port_error(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_ports::contract_tests;
    use std::collections::BTreeSet;

    #[test]
    fn satisfies_shared_artifact_repository_contract() {
        let store = SqliteStore::in_memory().expect("test setup");

        contract_tests::assert_artifact_repository_round_trip(&store);
    }

    #[test]
    fn satisfies_shared_event_log_contract() {
        let store = SqliteStore::in_memory().expect("test setup");

        contract_tests::assert_event_log_round_trip(&store);
    }

    #[test]
    fn satisfies_shared_chunk_repository_contract() {
        let store = SqliteStore::in_memory().expect("test setup");

        contract_tests::assert_chunk_repository_round_trip(&store);
    }

    #[test]
    fn satisfies_shared_card_repository_contract() {
        let store = SqliteStore::in_memory().expect("test setup");

        contract_tests::assert_card_repository_round_trip(&store);
    }

    #[test]
    fn satisfies_shared_evidence_repository_contract() {
        let store = SqliteStore::in_memory().expect("test setup");

        contract_tests::assert_evidence_repository_round_trip(&store);
    }

    fn artifact(id: u64) -> Artifact {
        Artifact {
            id: ArtifactId::new(id),
            title: format!("artifact {id}"),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
        }
    }

    fn registered(event_id: u64, sequence: u64, artifact_id: u64) -> DomainEventEnvelope {
        DomainEventEnvelope {
            id: EventId::new(event_id),
            sequence: SequenceNumber::new(sequence),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(artifact_id),
                title: format!("artifact {artifact_id}"),
            },
        }
    }

    #[test]
    fn migrations_are_idempotent() {
        let directory = tempfile::tempdir().expect("test setup");
        let path = directory.path().join("store.db");

        SqliteStore::open(&path).expect("test setup");
        SqliteStore::open(&path).expect("test setup");

        let connection = Connection::open(path).expect("test setup");
        let version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .expect("test setup");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        for table in ["chunks", "cards", "card_claims", "evidence"] {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .expect("table lookup");
            assert_eq!(count, 1, "{table} table should exist");
        }
    }

    #[test]
    fn artifact_put_get_and_missing() {
        let store = SqliteStore::in_memory().expect("test setup");
        assert_eq!(
            ArtifactRepository::get(&store, ArtifactId::new(9)).expect("missing artifact lookup"),
            None
        );

        let artifact = artifact(1);
        ArtifactRepository::put(&store, artifact.clone()).expect("test setup");

        assert_eq!(
            ArtifactRepository::get(&store, ArtifactId::new(1)).expect("stored artifact lookup"),
            Some(artifact)
        );
    }

    #[test]
    fn artifact_relationship_sets_round_trip() {
        let store = SqliteStore::in_memory().expect("test setup");
        let mut artifact = artifact(1);
        artifact
            .chunk_ids
            .extend([ChunkId::new(10), ChunkId::new(11)]);
        artifact.card_ids.extend([CardId::new(20), CardId::new(21)]);
        artifact
            .claim_ids
            .extend([ClaimId::new(30), ClaimId::new(31)]);
        artifact
            .evidence_ids
            .extend([EvidenceId::new(40), EvidenceId::new(41)]);

        ArtifactRepository::put(&store, artifact.clone()).expect("test setup");

        assert_eq!(
            ArtifactRepository::get(&store, ArtifactId::new(1)).expect("stored artifact lookup"),
            Some(artifact)
        );
    }

    #[test]
    fn brain_state_round_trips_and_lists_deterministically() {
        let store = SqliteStore::in_memory().expect("test setup");
        let late = Chunk {
            id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 2,
            text: "late".to_string(),
        };
        let early = Chunk {
            id: ChunkId::new(11),
            artifact_id: ArtifactId::new(1),
            order: 1,
            text: "early".to_string(),
        };
        let card = Card {
            id: CardId::new(20),
            artifact_id: ArtifactId::new(1),
            title: "card".to_string(),
            body: "body".to_string(),
            claim_ids: [ClaimId::new(5), ClaimId::new(3)].into(),
        };
        let evidence = Evidence {
            id: EvidenceId::new(30),
            artifact_id: ArtifactId::new(1),
            claim_id: Some(ClaimId::new(3)),
            kind: EvidenceKind::FileSpan {
                path: "notes.md".to_string(),
                range: ContentRange { start: 3, end: 8 },
                content_hash: "sha256:abc".to_string(),
            },
            excerpt: "grounded excerpt".to_string(),
            observed_at: LogicalTick::new(4),
        };

        ChunkRepository::put(&store, late.clone()).expect("late chunk put");
        ChunkRepository::put(&store, early.clone()).expect("early chunk put");
        CardRepository::put(&store, card.clone()).expect("card put");
        EvidenceRepository::put(&store, evidence.clone()).expect("evidence put");

        assert_eq!(
            ChunkRepository::list_for_artifact(&store, ArtifactId::new(1)).expect("chunk list"),
            vec![early.clone(), late.clone()]
        );
        assert_eq!(
            ChunkRepository::get(&store, late.id).expect("chunk get"),
            Some(late)
        );
        assert_eq!(
            CardRepository::list_for_artifact(&store, ArtifactId::new(1)).expect("card list"),
            vec![card.clone()]
        );
        assert_eq!(
            CardRepository::get(&store, card.id).expect("card get"),
            Some(card)
        );
        assert_eq!(
            EvidenceRepository::list_for_artifact(&store, ArtifactId::new(1))
                .expect("evidence list"),
            vec![evidence.clone()]
        );
        assert_eq!(
            EvidenceRepository::get(&store, evidence.id).expect("evidence get"),
            Some(evidence)
        );
    }

    #[test]
    fn evidence_kind_persists_without_domain_serde() {
        let store = SqliteStore::in_memory().expect("test setup");
        let command = Evidence {
            id: EvidenceId::new(31),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::CommandOutput {
                harness_run: HarnessRunId::new(8),
                stream: OutputStream::Stderr,
                blob: BlobId::new(9),
            },
            excerpt: "stderr excerpt".to_string(),
            observed_at: LogicalTick::new(5),
        };
        let validation = Evidence {
            id: EvidenceId::new(32),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::Validation {
                report_id: ValidationReportId::new(10),
            },
            excerpt: "validation excerpt".to_string(),
            observed_at: LogicalTick::new(6),
        };

        EvidenceRepository::put(&store, command.clone()).expect("command evidence put");
        EvidenceRepository::put(&store, validation.clone()).expect("validation evidence put");

        assert_eq!(
            EvidenceRepository::list_for_artifact(&store, ArtifactId::new(1))
                .expect("evidence list"),
            vec![command, validation]
        );
    }

    #[test]
    fn event_append_scan_order_and_filter() {
        let store = SqliteStore::in_memory().expect("test setup");
        let first = registered(1, 1, 7);
        let second = DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::TaskOpened {
                task_id: TaskId::new(99),
                title: "task".to_string(),
                priority: TaskPriority::High,
                artifact_id: Some(ArtifactId::new(7)),
            },
        };
        let third = DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::ChunkRegistered {
                chunk_id: ChunkId::new(8),
                artifact_id: ArtifactId::new(7),
                order: 0,
                text: "chunk".to_string(),
            },
        };
        let out_of_order = DomainEventEnvelope {
            id: EventId::new(5),
            sequence: SequenceNumber::new(5),
            event: DomainEvent::TickObserved {
                at: LogicalTick::new(1),
            },
        };

        store.append(first.clone()).expect("test setup");
        store.append(second.clone()).expect("test setup");
        store.append(third.clone()).expect("test setup");
        assert!(matches!(
            store.append(out_of_order),
            Err(PortError::Conflict { .. })
        ));

        assert_eq!(
            store
                .scan(EventFilter { artifact_id: None })
                .expect("full event scan"),
            vec![first.clone(), second.clone(), third.clone()]
        );
        assert_eq!(
            store
                .scan(EventFilter {
                    artifact_id: Some(ArtifactId::new(7)),
                })
                .expect("filtered event scan"),
            vec![first, second, third]
        );
    }

    #[test]
    fn artifact_filter_includes_evidence_and_search_events() {
        let store = SqliteStore::in_memory().expect("test setup");
        let evidence = DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: EvidenceId::new(40),
                artifact_id: ArtifactId::new(7),
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: "notes.md".to_string(),
                    range: ContentRange { start: 1, end: 4 },
                    content_hash: "sha256:notes".to_string(),
                },
                excerpt: "excerpt".to_string(),
                observed_at: LogicalTick::new(1),
            },
        };
        let search = DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::SearchCompleted {
                artifact_id: ArtifactId::new(7),
                cards_added: 2,
            },
        };
        let unrelated = registered(3, 3, 9);

        store.append(evidence.clone()).expect("evidence append");
        store.append(search.clone()).expect("search append");
        store.append(unrelated).expect("unrelated append");

        assert_eq!(
            store
                .scan(EventFilter {
                    artifact_id: Some(ArtifactId::new(7)),
                })
                .expect("filtered event scan"),
            vec![evidence, search]
        );
    }

    #[test]
    fn duplicate_event_id_or_sequence_conflicts() {
        let store = SqliteStore::in_memory().expect("test setup");
        store.append(registered(1, 1, 1)).expect("test setup");

        assert!(matches!(
            store.append(registered(1, 2, 1)),
            Err(PortError::Conflict { .. })
        ));
        assert!(matches!(
            store.append(registered(2, 1, 1)),
            Err(PortError::Conflict { .. })
        ));
    }

    #[test]
    fn append_rejects_swapped_existing_event_rows() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        {
            let connection = store.lock()?;
            connection
                .execute(
                    "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 2, 'artifact_registered', 1, ?1, 2)",
                    params![r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"first"}"#],
                )
                .map_err(to_port_error)?;
            connection
                .execute(
                    "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (2, 1, 'artifact_registered', 1, ?1, 2)",
                    params![r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"second"}"#],
                )
                .map_err(to_port_error)?;
        }
        assert!(matches!(
            store.append(registered(3, 3, 1)),
            Err(PortError::Conflict { .. })
        ));
        Ok(())
    }
    #[test]
    fn migration_rejects_event_metadata_mismatch() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("mismatched-metadata.db");
        {
            let connection = Connection::open(&path)?;
            connection.execute_batch(
                "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (2);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL,
                     payload_version INTEGER NOT NULL
                 );
                 INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'artifact_registered', NULL, '{\"event_kind\":\"artifact_registered\",\"artifact_id\":1,\"title\":\"artifact\"}', 2);",
            )?;
        }

        assert!(matches!(
            SqliteStore::open(&path),
            Err(PortError::Internal { .. })
        ));
        Ok(())
    }

    #[test]
    fn legacy_migration_rejects_lossy_existing_payloads() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("lossy-legacy.db");
        {
            let connection = Connection::open(&path)?;
            connection.execute_batch(
                "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (1);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL
                 );
                 INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json)
                 VALUES (1, 1, 'chunk_registered', 1, '{\"event_kind\":\"chunk_registered\",\"chunk_id\":1,\"artifact_id\":1,\"order\":0}');",
            )?;
        }

        assert!(matches!(
            SqliteStore::open(&path),
            Err(PortError::InvalidInput { .. })
        ));
        Ok(())
    }

    #[test]
    fn legacy_event_rows_migrate_and_reject_lossy_payloads()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("legacy.db");
        {
            let connection = Connection::open(&path)?;
            connection.execute_batch(
                "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (1);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL
                 );",
            )?;
        }

        let store = SqliteStore::open(&path)?;
        {
            let connection = store.lock()?;
            connection.execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'artifact_registered', 1, ?1, 1)",
                params![r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"legacy"}"#],
            )?;
        }
        assert_eq!(
            store.scan(EventFilter { artifact_id: None })?,
            vec![DomainEventEnvelope {
                id: EventId::new(1),
                sequence: SequenceNumber::new(1),
                event: DomainEvent::ArtifactRegistered {
                    artifact_id: ArtifactId::new(1),
                    title: "legacy".to_string(),
                },
            }]
        );

        {
            let connection = store.lock()?;
            connection.execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (2, 2, 'relation_created', NULL, ?1, 1)",
                params![r#"{"event_kind":"relation_created","relation_id":7}"#],
            )?;
        }
        assert!(matches!(
            store.scan(EventFilter { artifact_id: None }),
            Err(PortError::InvalidInput { .. })
        ));

        let connection = store.lock()?;
        let has_payload_version: i64 = connection.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('domain_events') WHERE name = 'payload_version'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(has_payload_version, 1);
        Ok(())
    }

    #[test]
    fn migration_rejects_non_nullable_artifact_column() -> Result<(), PortError> {
        let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
        connection
            .execute_batch(
                "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (2);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER NOT NULL,
                     payload_json TEXT NOT NULL,
                     payload_version INTEGER NOT NULL
                 );",
            )
            .map_err(to_port_error)?;

        assert!(matches!(
            migrate(&mut connection),
            Err(PortError::Internal { message }) if message.contains("artifact_id")
        ));
        Ok(())
    }

    #[test]
    fn legacy_migration_rejects_noncontiguous_event_rows() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("malformed-legacy.db");
        {
            let connection = Connection::open(&path)?;
            connection.execute_batch(
                "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (1);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL
                 );
                 INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json)
                 VALUES (9, 9, 'artifact_registered', 1, '{\"event_kind\":\"artifact_registered\",\"artifact_id\":1,\"title\":\"legacy\"}');",
            )?;
        }

        assert!(matches!(
            SqliteStore::open(&path),
            Err(PortError::Internal { .. })
        ));
        Ok(())
    }

    #[test]
    fn malformed_v2_payload_is_rejected_without_defaults() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        {
            let connection = store.lock()?;
            connection
                .execute(
                    "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'chunk_registered', 1, ?1, 2)",
                    params![r#"{"event_kind":"chunk_registered","chunk_id":1,"artifact_id":1,"order":0}"#],
                )
                .map_err(to_port_error)?;
        }
        assert!(matches!(
            store.scan(EventFilter { artifact_id: None }),
            Err(PortError::Internal { .. })
        ));
        Ok(())
    }

    #[test]
    fn strict_v2_payloads_reject_missing_and_unknown_fields() -> Result<(), PortError> {
        let missing_warnings = SqliteStore::in_memory()?;
        {
            let connection = missing_warnings.lock()?;
            connection
                .execute(
                    "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'validation_report_created', NULL, ?1, 2)",
                    params![r#"{"event_kind":"validation_report_created","report_id":1,"task_id":null,"passed":true}"#],
                )
                .map_err(to_port_error)?;
        }
        assert!(matches!(
            missing_warnings.scan(EventFilter { artifact_id: None }),
            Err(PortError::Internal { .. })
        ));

        let unknown_field = SqliteStore::in_memory()?;
        {
            let connection = unknown_field.lock()?;
            connection
                .execute(
                    "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'artifact_registered', 1, ?1, 2)",
                    params![r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"artifact","unexpected":true}"#],
                )
                .map_err(to_port_error)?;
        }
        assert!(matches!(
            unknown_field.scan(EventFilter { artifact_id: None }),
            Err(PortError::Internal { .. })
        ));
        let unknown_nested_field = SqliteStore::in_memory()?;
        {
            let connection = unknown_nested_field.lock()?;
            connection
                .execute(
                    "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'relation_created', NULL, ?1, 2)",
                    params![r#"{"event_kind":"relation_created","relation_id":1,"source":{"kind":"artifact","artifact_id":1,"unexpected":true},"kind":"supports","target":{"kind":"artifact","artifact_id":2},"evidence_id":null,"confidence_milli":1000}"#],
                )
                .map_err(to_port_error)?;
        }
        assert!(matches!(
            unknown_nested_field.scan(EventFilter { artifact_id: None }),
            Err(PortError::Internal { .. })
        ));

        let mismatched_metadata = SqliteStore::in_memory()?;
        {
            let connection = mismatched_metadata.lock()?;
            connection
                .execute(
                    "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'artifact_registered', NULL, ?1, 2)",
                    params![r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"artifact"}"#],
                )
                .map_err(to_port_error)?;
        }
        assert!(matches!(
            mismatched_metadata.scan(EventFilter { artifact_id: None }),
            Err(PortError::Internal { .. })
        ));

        Ok(())
    }

    #[test]
    fn fresh_schema_writes_payload_version_two() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        store.append(registered(1, 1, 1))?;
        let connection = store.lock()?;
        let version: i64 = connection
            .query_row(
                "SELECT payload_version FROM domain_events WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .map_err(to_port_error)?;
        assert_eq!(version, 2);
        Ok(())
    }
}
