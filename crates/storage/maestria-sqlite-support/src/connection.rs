use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use maestria_ports::PortError;
use rusqlite::Connection;

use crate::error::to_port_error;

/// Opens a SQLite database with the shared connection policy: a five-second
/// busy timeout, WAL journal mode, NORMAL synchronous mode, in-memory temp
/// store, 64MB cache size, and 256MB memory mapping. Schema migration stays
/// with the owning crate.
pub fn open_connection(path: impl AsRef<Path>) -> Result<Connection, PortError> {
    let connection = Connection::open(path).map_err(to_port_error)?;
    configure_connection(&connection, false)?;
    Ok(connection)
}

/// Opens an in-memory SQLite database with the same busy-timeout and
/// performance configuration as [`open_connection`].
pub fn open_in_memory_connection() -> Result<Connection, PortError> {
    let connection = Connection::open_in_memory().map_err(to_port_error)?;
    configure_connection(&connection, true)?;
    Ok(connection)
}

fn configure_connection(connection: &Connection, is_memory: bool) -> Result<(), PortError> {
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(to_port_error)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(to_port_error)?;
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(to_port_error)?;
    connection
        .pragma_update(None, "temp_store", "MEMORY")
        .map_err(to_port_error)?;
    connection
        .pragma_update(None, "cache_size", -64000_i64)
        .map_err(to_port_error)?;
    if !is_memory {
        connection
            .pragma_update(None, "mmap_size", 268435456_i64)
            .map_err(to_port_error)?;
    }
    Ok(())
}

/// Acquires the connection mutex, mapping a poisoned lock to an internal
/// error carrying `context`.
pub fn lock_connection<'a>(
    connection: &'a Mutex<Connection>,
    context: &'static str,
) -> Result<MutexGuard<'a, Connection>, PortError> {
    connection
        .lock()
        .map_err(|_| PortError::internal(context, "connection mutex is poisoned"))
}

/// Runs `operation` against the connection under the mutex, mapping lock
/// poisoning through [`lock_connection`].
pub fn with_connection<T>(
    connection: &Mutex<Connection>,
    context: &'static str,
    operation: impl FnOnce(&mut Connection) -> Result<T, PortError>,
) -> Result<T, PortError> {
    let mut guard = lock_connection(connection, context)?;
    operation(&mut guard)
}
