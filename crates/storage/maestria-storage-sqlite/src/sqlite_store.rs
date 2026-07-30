use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use maestria_domain::{ArtifactId, CardId, ChunkId, EvidenceId};
use maestria_ports::{
    EffectJournal, EffectJournalEntry, EffectJournalIntent, EffectJournalStatus, HarnessOutcome,
    HarnessRunId, PortError,
};
use rusqlite::{Connection, Error, ErrorCode, OpenFlags, Transaction, params};

use crate::{repositories, schema::migrate};

/// SQLite-backed implementation of artifact metadata and the domain event log.
pub struct SqliteStore {
    connection: std::sync::Mutex<Connection>,
}

impl SqliteStore {
    /// Open a SQLite database file and apply idempotent schema migrations.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, PortError> {
        let mut connection = Connection::open(path).map_err(to_port_error)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(to_port_error)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(to_port_error)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: std::sync::Mutex::new(connection),
        })
    }

    /// Open an existing database for replay without acquiring migration or
    /// journal-mode write locks.
    pub fn open_read_only(path: impl AsRef<std::path::Path>) -> Result<Self, PortError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(to_port_error)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(to_port_error)?;
        Ok(Self {
            connection: std::sync::Mutex::new(connection),
        })
    }

    /// Open an in-memory SQLite database and apply idempotent schema migrations.
    pub fn in_memory() -> Result<Self, PortError> {
        let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(to_port_error)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(to_port_error)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: std::sync::Mutex::new(connection),
        })
    }

    /// Export all validated learned-sparse shadow observations as typed JSON.
    pub fn export_learned_sparse_observations(&self) -> Result<String, PortError> {
        let limit =
            NonZeroUsize::new(i64::MAX as usize).ok_or_else(|| PortError::InternalContext {
                context: "learned-sparse shadow export limit",
                source: "platform usize has no positive value".to_string(),
            })?;
        let connection = self.lock()?;
        let observations = repositories::learned_sparse_observation_repo::scan(&connection, limit)?;
        serde_json::to_string(&observations).map_err(json_error)
    }

    /// Replace learned-sparse shadow observations from validated typed JSON.
    pub fn import_learned_sparse_observations(&self, input: &str) -> Result<(), PortError> {
        let observations = serde_json::from_str(input).map_err(json_error)?;
        let mut connection = self.lock()?;
        repositories::learned_sparse_observation_repo::replace(&mut connection, observations)
    }
    /// Remove projection rows whose IDs are absent from replayed state.
    ///
    /// The operation runs in one SQLite transaction and only deletes IDs not
    /// present in the typed keep sets. Artifact mappings are removed before
    /// stale artifact parents, while entity upserts happen separately in the
    /// daemon after this cleanup so missing valid rows are still repaired.
    pub fn remove_stale_projection_rows(
        &self,
        artifact_ids: &BTreeSet<ArtifactId>,
        chunk_ids: &BTreeSet<ChunkId>,
        card_ids: &BTreeSet<CardId>,
        evidence_ids: &BTreeSet<EvidenceId>,
    ) -> Result<(), PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        remove_stale_projection_mappings(
            &transaction,
            "SELECT DISTINCT artifact_id FROM artifact_chunks",
            "DELETE FROM artifact_chunks WHERE artifact_id = ?1",
            artifact_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_mappings(
            &transaction,
            "SELECT DISTINCT artifact_id FROM artifact_cards",
            "DELETE FROM artifact_cards WHERE artifact_id = ?1",
            artifact_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_mappings(
            &transaction,
            "SELECT DISTINCT artifact_id FROM artifact_claims",
            "DELETE FROM artifact_claims WHERE artifact_id = ?1",
            artifact_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_mappings(
            &transaction,
            "SELECT DISTINCT artifact_id FROM artifact_evidences",
            "DELETE FROM artifact_evidences WHERE artifact_id = ?1",
            artifact_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_ids(
            &transaction,
            "SELECT id FROM artifacts",
            "DELETE FROM artifacts WHERE id = ?1",
            artifact_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_ids(
            &transaction,
            "SELECT id FROM chunks",
            "DELETE FROM chunks WHERE id = ?1",
            chunk_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_ids(
            &transaction,
            "SELECT id FROM cards",
            "DELETE FROM cards WHERE id = ?1",
            card_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_ids(
            &transaction,
            "SELECT id FROM evidence",
            "DELETE FROM evidence WHERE id = ?1",
            evidence_ids.iter().map(|id| id.value()),
        )?;
        transaction.commit().map_err(to_port_error)
    }

    pub(crate) fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, PortError> {
        self.connection
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "sqlite connection lock poisoned",
                source: "connection mutex is poisoned".to_string(),
            })
    }
}

fn remove_stale_projection_ids(
    transaction: &Transaction<'_>,
    select_sql: &'static str,
    delete_sql: &'static str,
    keep_ids: impl Iterator<Item = u64>,
) -> Result<(), PortError> {
    let keep_ids = keep_ids.collect::<BTreeSet<_>>();
    let mut statement = transaction.prepare(select_sql).map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let mut stale_ids = Vec::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let stored_id = row.get::<_, i64>(0).map_err(to_port_error)?;
        if !keep_ids.contains(&i64_to_u64(stored_id)?) {
            stale_ids.push(stored_id);
        }
    }
    drop(rows);
    drop(statement);

    for stale_id in stale_ids {
        transaction
            .execute(delete_sql, params![stale_id])
            .map_err(to_port_error)?;
    }
    Ok(())
}
fn remove_stale_projection_mappings(
    transaction: &Transaction<'_>,
    select_sql: &'static str,
    delete_sql: &'static str,
    keep_artifact_ids: impl Iterator<Item = u64>,
) -> Result<(), PortError> {
    remove_stale_projection_ids(transaction, select_sql, delete_sql, keep_artifact_ids)
}

impl EffectJournal for SqliteStore {
    fn record_intent(&self, intent: EffectJournalIntent) -> Result<EffectJournalEntry, PortError> {
        let mut connection = self.lock()?;
        repositories::effect_journal_repo::record_intent(&mut connection, intent)
    }

    fn record_started(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::record_started(&connection, run_id, generation)
    }
    fn claim_feedback(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::claim_feedback(&connection, run_id, generation)
    }

    fn claim_feedback_with_outcome(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        outcome: HarnessOutcome,
    ) -> Result<(), PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::claim_feedback_with_outcome(
            &connection,
            run_id,
            generation,
            Some(&outcome),
        )
    }

    fn feedback_outcome(
        &self,
        run_id: HarnessRunId,
        generation: u64,
    ) -> Result<Option<HarnessOutcome>, PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::feedback_outcome(&connection, run_id, generation)
    }

    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::record_terminal(&connection, run_id, generation, status)
    }

    fn scan_in_flight(&self) -> Result<Vec<EffectJournalEntry>, PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::scan_in_flight(&connection)
    }

    fn is_feedback_accepted(
        &self,
        run_id: HarnessRunId,
        generation: u64,
    ) -> Result<bool, PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::is_feedback_accepted(&connection, run_id, generation)
    }
    fn is_current(&self, run_id: HarnessRunId, generation: u64) -> Result<bool, PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::is_current(&connection, run_id, generation)
    }
}

pub(crate) fn to_port_error(error: Error) -> PortError {
    PortError::DownstreamContext {
        context: "sqlite database query failed",
        source: error.to_string(),
    }
}

pub(crate) fn map_append_error(error: Error) -> PortError {
    if let Error::SqliteFailure(failure, _) = &error
        && failure.code == ErrorCode::ConstraintViolation
    {
        return PortError::Conflict {
            message: "domain event id or sequence already exists".to_string(),
        };
    }
    to_port_error(error)
}

pub(crate) fn json_error(error: serde_json::Error) -> PortError {
    PortError::InternalContext {
        context: "event payload serialization failed",
        source: error.to_string(),
    }
}

pub(crate) fn u64_to_i64(value: u64) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| PortError::InvalidInputContext {
        context: "identifier exceeds sqlite INTEGER range",
        source: value.to_string(),
    })
}

pub(crate) fn optional_u64_to_i64(value: Option<u64>) -> Result<Option<i64>, PortError> {
    value.map(u64_to_i64).transpose()
}

pub(crate) fn i64_to_u64(value: i64) -> Result<u64, PortError> {
    u64::try_from(value).map_err(|_| PortError::InternalContext {
        context: "stored identifier is negative",
        source: value.to_string(),
    })
}

pub(crate) fn i64_to_u32(value: i64) -> Result<u32, PortError> {
    u32::try_from(value).map_err(|_| PortError::InternalContext {
        context: "stored chunk order is outside u32 range",
        source: value.to_string(),
    })
}

pub(crate) fn optional_i64_to_u64(value: Option<i64>) -> Result<Option<u64>, PortError> {
    value.map(i64_to_u64).transpose()
}
