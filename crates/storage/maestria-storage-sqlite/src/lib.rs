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
    Artifact, ArtifactId, BlobId, CardId, ChunkId, ClaimId, ClaimStatus, ContentRange, DomainEvent,
    DomainEventEnvelope, EventId, EvidenceId, EvidenceKind, HarnessRunId, LogicalTick,
    OutputStream, SequenceNumber, TaskId, TaskPriority, TaskStatus, ValidationReportId,
};
use maestria_ports::{ArtifactRepository, EventFilter, EventLog, PortError};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

const CURRENT_SCHEMA_VERSION: i64 = 1;

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

impl EventLog for SqliteStore {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        let record = StoredEvent::from_domain(&event)?;
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json)\n                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    u64_to_i64(record.id)?,
                    u64_to_i64(record.sequence)?,
                    record.kind,
                    optional_u64_to_i64(record.artifact_id)?,
                    record.payload_json,
                ],
            )
            .map(|_| ())
            .map_err(map_append_error)
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        let connection = self.lock()?;
        let mut events = Vec::new();

        if let Some(artifact_id) = filter.artifact_id {
            let mut statement = connection
                .prepare(
                    "SELECT id, sequence, event_kind, artifact_id, payload_json\n                     FROM domain_events\n                     WHERE artifact_id = ?1\n                     ORDER BY sequence ASC",
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
                    "SELECT id, sequence, event_kind, artifact_id, payload_json\n                     FROM domain_events\n                     ORDER BY sequence ASC",
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
    transaction
        .execute_batch(
            "PRAGMA foreign_keys = ON;\n             CREATE TABLE IF NOT EXISTS schema_version (\n                 version INTEGER NOT NULL PRIMARY KEY,\n                 applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP\n             );\n             CREATE TABLE IF NOT EXISTS artifacts (\n                 id INTEGER NOT NULL PRIMARY KEY,\n                 title TEXT NOT NULL\n             );\n             CREATE TABLE IF NOT EXISTS artifact_chunks (\n                 artifact_id INTEGER NOT NULL,\n                 related_id INTEGER NOT NULL,\n                 PRIMARY KEY (artifact_id, related_id),\n                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE\n             );\n             CREATE TABLE IF NOT EXISTS artifact_cards (\n                 artifact_id INTEGER NOT NULL,\n                 related_id INTEGER NOT NULL,\n                 PRIMARY KEY (artifact_id, related_id),\n                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE\n             );\n             CREATE TABLE IF NOT EXISTS artifact_claims (\n                 artifact_id INTEGER NOT NULL,\n                 related_id INTEGER NOT NULL,\n                 PRIMARY KEY (artifact_id, related_id),\n                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE\n             );\n             CREATE TABLE IF NOT EXISTS artifact_evidences (\n                 artifact_id INTEGER NOT NULL,\n                 related_id INTEGER NOT NULL,\n                 PRIMARY KEY (artifact_id, related_id),\n                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE\n             );\n             CREATE TABLE IF NOT EXISTS domain_events (\n                 id INTEGER NOT NULL PRIMARY KEY,\n                 sequence INTEGER NOT NULL UNIQUE,\n                 event_kind TEXT NOT NULL,\n                 artifact_id INTEGER,\n                 payload_json TEXT NOT NULL\n             );\n             CREATE INDEX IF NOT EXISTS idx_domain_events_artifact_sequence\n                 ON domain_events(artifact_id, sequence);\n             INSERT OR IGNORE INTO schema_version (version) VALUES (1);",
        )
        .map_err(to_port_error)?;

    let version = transaction
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(to_port_error)?;
    if version != CURRENT_SCHEMA_VERSION {
        return Err(PortError::Internal {
            message: format!(
                "unsupported sqlite schema version {version}; expected {CURRENT_SCHEMA_VERSION}"
            ),
        });
    }

    transaction.commit().map_err(to_port_error)
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

#[derive(Debug)]
struct StoredEvent {
    id: u64,
    sequence: u64,
    kind: &'static str,
    artifact_id: Option<u64>,
    payload_json: String,
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
        })
    }

    fn into_domain(self) -> Result<DomainEventEnvelope, PortError> {
        let payload: StoredEventPayload =
            serde_json::from_str(&self.payload_json).map_err(json_error)?;
        if payload.kind() != self.kind {
            return Err(PortError::Internal {
                message: format!(
                    "stored event kind mismatch: column {}, payload {}",
                    self.kind,
                    payload.kind()
                ),
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
#[serde(tag = "event_kind", rename_all = "snake_case")]
enum StoredEventPayload {
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
        artifact_id: u64,
        claim_id: Option<u64>,
        evidence_kind: StoredEvidenceKind,
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
        #[serde(default)]
        evidence_ids: Vec<u64>,
        #[serde(default)]
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
        #[serde(default)]
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
            } => Self::ChunkRegistered {
                chunk_id: chunk_id.value(),
                artifact_id: artifact_id.value(),
                order: *order,
            },
            DomainEvent::CardCreated {
                card_id,
                artifact_id,
            } => Self::CardCreated {
                card_id: card_id.value(),
                artifact_id: artifact_id.value(),
            },
            DomainEvent::ClaimCreated {
                claim_id,
                artifact_id,
            } => Self::ClaimCreated {
                claim_id: claim_id.value(),
                artifact_id: artifact_id.value(),
            },
            DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                kind,
            } => Self::EvidenceRecorded {
                evidence_id: evidence_id.value(),
                artifact_id: artifact_id.value(),
                claim_id: claim_id.map(|id| id.value()),
                evidence_kind: StoredEvidenceKind::from_domain(kind),
            },
            DomainEvent::TaskOpened {
                task_id,
                title,
                priority,
            } => Self::TaskOpened {
                task_id: task_id.value(),
                title: title.clone(),
                priority: StoredTaskPriority::from_domain(*priority),
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
            DomainEvent::RelationCreated { relation_id } => Self::RelationCreated {
                relation_id: relation_id.value(),
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
            } => DomainEvent::ChunkRegistered {
                chunk_id: ChunkId::new(chunk_id),
                artifact_id: ArtifactId::new(artifact_id),
                order,
            },
            Self::CardCreated {
                card_id,
                artifact_id,
            } => DomainEvent::CardCreated {
                card_id: CardId::new(card_id),
                artifact_id: ArtifactId::new(artifact_id),
            },
            Self::ClaimCreated {
                claim_id,
                artifact_id,
            } => DomainEvent::ClaimCreated {
                claim_id: ClaimId::new(claim_id),
                artifact_id: ArtifactId::new(artifact_id),
            },
            Self::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                evidence_kind,
            } => DomainEvent::EvidenceRecorded {
                evidence_id: EvidenceId::new(evidence_id),
                artifact_id: ArtifactId::new(artifact_id),
                claim_id: claim_id.map(ClaimId::new),
                kind: evidence_kind.into_domain(),
            },
            Self::TaskOpened {
                task_id,
                title,
                priority,
            } => DomainEvent::TaskOpened {
                task_id: TaskId::new(task_id),
                title,
                priority: priority.into_domain(),
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
            Self::RelationCreated { relation_id } => DomainEvent::RelationCreated {
                relation_id: maestria_domain::RelationId::new(relation_id),
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
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
    if let rusqlite::Error::SqliteFailure(failure, _) = &error {
        if failure.code == ErrorCode::ConstraintViolation {
            return PortError::Conflict {
                message: "domain event id or sequence already exists".to_string(),
            };
        }
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
    }

    #[test]
    fn artifact_put_get_and_missing() {
        let store = SqliteStore::in_memory().expect("test setup");
        assert_eq!(
            store
                .get(ArtifactId::new(9))
                .expect("missing artifact lookup"),
            None
        );

        let artifact = artifact(1);
        store.put(artifact.clone()).expect("test setup");

        assert_eq!(
            store
                .get(ArtifactId::new(1))
                .expect("stored artifact lookup"),
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

        store.put(artifact.clone()).expect("test setup");

        assert_eq!(
            store
                .get(ArtifactId::new(1))
                .expect("stored artifact lookup"),
            Some(artifact)
        );
    }

    #[test]
    fn event_append_scan_order_and_filter() {
        let store = SqliteStore::in_memory().expect("test setup");
        let first = registered(1, 2, 7);
        let second = DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::TaskOpened {
                task_id: TaskId::new(99),
                title: "task".to_string(),
                priority: TaskPriority::High,
            },
        };
        let third = DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::ChunkRegistered {
                chunk_id: ChunkId::new(8),
                artifact_id: ArtifactId::new(7),
                order: 0,
            },
        };

        store.append(first.clone()).expect("test setup");
        store.append(second.clone()).expect("test setup");
        store.append(third.clone()).expect("test setup");

        assert_eq!(
            store
                .scan(EventFilter { artifact_id: None })
                .expect("full event scan"),
            vec![second, first.clone(), third.clone()]
        );
        assert_eq!(
            store
                .scan(EventFilter {
                    artifact_id: Some(ArtifactId::new(7)),
                })
                .expect("filtered event scan"),
            vec![first, third]
        );
    }

    #[test]
    fn artifact_filter_includes_evidence_and_search_events() {
        let store = SqliteStore::in_memory().expect("test setup");
        let evidence = DomainEventEnvelope {
            id: EventId::new(10),
            sequence: SequenceNumber::new(10),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: EvidenceId::new(40),
                artifact_id: ArtifactId::new(7),
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: "notes.md".to_string(),
                    range: ContentRange { start: 1, end: 4 },
                    content_hash: "sha256:notes".to_string(),
                },
            },
        };
        let search = DomainEventEnvelope {
            id: EventId::new(11),
            sequence: SequenceNumber::new(11),
            event: DomainEvent::SearchCompleted {
                artifact_id: ArtifactId::new(7),
                cards_added: 2,
            },
        };
        let unrelated = registered(12, 12, 9);

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
}
