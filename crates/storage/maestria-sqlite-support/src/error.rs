use maestria_ports::PortError;
use rusqlite::{Error, ErrorCode};

/// Maps a [`rusqlite::Error`] to the canonical [`PortError`] classification:
/// constraint violations become [`PortError::Conflict`] (callers rely on this
/// for upsert-vs-insert and uniqueness semantics), everything else is a
/// downstream error attributed to the SQLite layer.
pub fn to_port_error(error: Error) -> PortError {
    if let Error::SqliteFailure(failure, _) = &error
        && failure.code == ErrorCode::ConstraintViolation
    {
        return PortError::Conflict {
            message: error.to_string(),
        };
    }
    PortError::downstream("sqlite database query failed", error.to_string())
}
