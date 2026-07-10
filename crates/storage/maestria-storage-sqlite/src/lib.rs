#![forbid(unsafe_code)]

//! SQLite-backed metadata and event-log adapter for Maestria.
//!
//! This crate intentionally keeps storage serialization at the port boundary:
//! domain types do not implement or depend on serde.

use maestria_ports::PortError;
use rusqlite::{Connection, Error, ErrorCode};

mod events;
mod payloads;
mod repositories;
mod schema;
use crate::schema::migrate;

/// SQLite-backed implementation of artifact metadata and the domain event log.
pub struct SqliteStore {
    connection: std::sync::Mutex<Connection>,
}

impl SqliteStore {
    /// Open a SQLite database file and apply idempotent schema migrations.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, PortError> {
        let mut connection = Connection::open(path).map_err(to_port_error)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: std::sync::Mutex::new(connection),
        })
    }

    /// Open an in-memory SQLite database and apply idempotent schema migrations.
    ///
    /// Useful for crate-local tests and short-lived adapters.
    pub fn in_memory() -> Result<Self, PortError> {
        let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: std::sync::Mutex::new(connection),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, PortError> {
        self.connection.lock().map_err(|_| PortError::Internal {
            message: "sqlite connection lock poisoned".to_string(),
        })
    }
}

fn to_port_error(error: Error) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
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
    PortError::Internal {
        message: format!("event payload serialization failed: {error}"),
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

#[cfg(test)]
mod tests;
