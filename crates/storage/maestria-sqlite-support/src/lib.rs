//! Shared SQLite plumbing for the storage crates.
//!
//! Every SQLite-backed projection (storage-sqlite, vector-sqlite,
//! graph-sqlite) opens connections, maps `rusqlite::Error` to [`maestria_ports::PortError`],
//! and converts u64 ids to SQLite INTEGER. This crate owns those conversions
//! once so error classification and integer-bound semantics cannot diverge
//! between projections (Rule 28: the layer producing an error defines it).
//!
//! Responsibility map:
//! - `connection`: open + busy-timeout + WAL + poisoned-lock plumbing.
//! - `db_retry`: shared `is_database_busy` matcher and retry constants.
//! - `error`: `rusqlite::Error` -> [`maestria_ports::PortError`] classification.
//! - `ids`: u64<->i64 id conversion and [`BindId`] SQL binding.
//! - `security`: shared default `security_json` literal for DDL.
mod connection;
mod db_retry;
mod error;
mod ids;
mod security;

pub use connection::{
    lock_connection, open_connection, open_in_memory_connection, with_connection,
};
pub use db_retry::{RETRY_ATTEMPTS, RETRY_DELAY, is_database_busy};
pub use error::to_port_error;
pub use ids::{
    BindId, i64_to_u32, i64_to_u64, i64_to_usize, optional_i64_to_u64, optional_u64_to_i64,
    u64_to_i64, usize_to_i64,
};
pub use security::DEFAULT_SECURITY_JSON;
