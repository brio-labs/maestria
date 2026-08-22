use maestria_ports::PortError;
use maestria_sqlite_support::lock_connection;
use rusqlite::{Connection, Error, ErrorCode, OpenFlags, OptionalExtension};

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
        // WAL + synchronous=NORMAL keeps commits safe against application
        // crashes while dropping the per-transaction fsync that dominated
        // batch-ingestion wall time; an OS-level power loss may discard the
        // most recent commits, and startup recovery re-drives the affected
        // artifacts from the durable event-log prefix.
        connection
            .pragma_update(None, "synchronous", "NORMAL")
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

    pub(crate) fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, PortError> {
        lock_connection(&self.connection, "sqlite connection lock poisoned")
    }

    pub(crate) fn with_transaction<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, PortError>,
    ) -> Result<T, PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        let result = f(&transaction)?;
        transaction.commit().map_err(to_port_error)?;
        Ok(result)
    }

    /// Latest durable domain-event id (0 for an empty log).
    ///
    /// Concrete-store API for startup reconciliation watermarking; not part
    /// of the [`maestria_ports::EventLog`] port.
    pub fn max_event_id(&self) -> Result<i64, PortError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT COALESCE(MAX(id), 0) FROM domain_events",
                [],
                |row| row.get(0),
            )
            .map_err(to_port_error)
    }

    /// Stored startup-reconciliation watermark, when one has been written.
    pub fn projection_watermark(&self) -> Result<Option<String>, PortError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT value FROM projection_meta WHERE key = 'reconcile_event_watermark'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(to_port_error)
    }

    /// Persist the startup-reconciliation watermark.
    pub fn set_projection_watermark(&self, value: &str) -> Result<(), PortError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO projection_meta(key, value) VALUES ('reconcile_event_watermark', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![value],
            )
            .map_err(to_port_error)?;
        Ok(())
    }

    /// Row counts of the entity projection tables (drift signal for startup
    /// reconciliation watermarking).
    pub fn projection_row_counts(&self) -> Result<(u64, u64, u64, u64), PortError> {
        let connection = self.lock()?;
        let count = |table: &str| -> Result<u64, PortError> {
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .map(|count| count.max(0) as u64)
                .map_err(|error| PortError::InternalContext {
                    context: "projection row count",
                    source: error.to_string(),
                })
        };
        Ok((
            count("artifacts")?,
            count("chunks")?,
            count("cards")?,
            count("evidence")?,
        ))
    }
}

pub(crate) use maestria_sqlite_support::{
    i64_to_u32, i64_to_u64, i64_to_usize, optional_i64_to_u64, optional_u64_to_i64, to_port_error,
    u64_to_i64, usize_to_i64,
};

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

pub(crate) fn row_str<'a>(row: &'a rusqlite::Row<'_>, idx: usize) -> Result<&'a str, PortError> {
    row.get_ref(idx)
        .map_err(to_port_error)?
        .as_str()
        .map_err(|err| {
            to_port_error(rusqlite::Error::FromSqlConversionFailure(
                idx,
                rusqlite::types::Type::Text,
                Box::new(err),
            ))
        })
}

pub(crate) fn row_opt_str<'a>(
    row: &'a rusqlite::Row<'_>,
    idx: usize,
) -> Result<Option<&'a str>, PortError> {
    let val = row.get_ref(idx).map_err(to_port_error)?;
    match val {
        rusqlite::types::ValueRef::Null => Ok(None),
        _ => val.as_str().map(Some).map_err(|err| {
            to_port_error(rusqlite::Error::FromSqlConversionFailure(
                idx,
                rusqlite::types::Type::Text,
                Box::new(err),
            ))
        }),
    }
}
