#![forbid(unsafe_code)]

//! SQLite-backed metadata and event-log adapter for Maestria.
//!
//! This crate intentionally keeps storage serialization at the port boundary:
//! domain types do not implement or depend on serde.

/// Responsibility map:
/// - `events`: module responsibility.
/// - `id_allocator`: module responsibility.
/// - `payloads`: module responsibility.
/// - `repositories`: module responsibility.
/// - `schema`: module responsibility.
/// - `schema_validation`: module responsibility.
use std::collections::BTreeSet;

use maestria_domain::{CardId, ChunkId, EvidenceId};
use maestria_ports::PortError;
use maestria_ports::{
    EffectJournal, EffectJournalEntry, EffectJournalIntent, EffectJournalStatus, HarnessRunId,
};
use rusqlite::{Connection, Error, ErrorCode, Transaction, params};

mod events;
mod id_allocator;
mod payloads;
mod repositories;
mod schema;
mod schema_validation;
use crate::schema::migrate;

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

    /// Remove child projection rows whose IDs are absent from replayed state.
    ///
    /// The operation runs in one SQLite transaction and only deletes IDs not
    /// present in the typed keep sets. Entity upserts happen separately in the
    /// daemon after this cleanup, so missing valid rows are still repaired.
    pub fn remove_stale_projection_children(
        &self,
        chunk_ids: &BTreeSet<ChunkId>,
        card_ids: &BTreeSet<CardId>,
        evidence_ids: &BTreeSet<EvidenceId>,
    ) -> Result<(), PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        remove_stale_projection_rows(
            &transaction,
            "SELECT id FROM chunks",
            "DELETE FROM chunks WHERE id = ?1",
            chunk_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_rows(
            &transaction,
            "SELECT id FROM cards",
            "DELETE FROM cards WHERE id = ?1",
            card_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_rows(
            &transaction,
            "SELECT id FROM evidence",
            "DELETE FROM evidence WHERE id = ?1",
            evidence_ids.iter().map(|id| id.value()),
        )?;
        transaction.commit().map_err(to_port_error)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, PortError> {
        self.connection
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "sqlite connection lock poisoned",
                source: "connection mutex is poisoned".to_string(),
            })
    }
}

fn remove_stale_projection_rows(
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

fn to_port_error(error: Error) -> PortError {
    PortError::DownstreamContext {
        context: "sqlite database query failed",
        source: error.to_string(),
    }
}

fn map_append_error(error: Error) -> PortError {
    if let Error::SqliteFailure(failure, _) = &error
        && failure.code == ErrorCode::ConstraintViolation
    {
        return PortError::Conflict {
            message: "domain event id or sequence already exists".to_string(),
        };
    }
    to_port_error(error)
}

fn json_error(error: serde_json::Error) -> PortError {
    PortError::InternalContext {
        context: "event payload serialization failed",
        source: error.to_string(),
    }
}

fn u64_to_i64(value: u64) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| PortError::InvalidInputContext {
        context: "identifier exceeds sqlite INTEGER range",
        source: value.to_string(),
    })
}

fn optional_u64_to_i64(value: Option<u64>) -> Result<Option<i64>, PortError> {
    value.map(u64_to_i64).transpose()
}

fn i64_to_u64(value: i64) -> Result<u64, PortError> {
    u64::try_from(value).map_err(|_| PortError::InternalContext {
        context: "stored identifier is negative",
        source: value.to_string(),
    })
}

fn i64_to_u32(value: i64) -> Result<u32, PortError> {
    u32::try_from(value).map_err(|_| PortError::InternalContext {
        context: "stored chunk order is outside u32 range",
        source: value.to_string(),
    })
}

fn optional_i64_to_u64(value: Option<i64>) -> Result<Option<u64>, PortError> {
    value.map(i64_to_u64).transpose()
}

#[cfg(test)]
mod tests;
