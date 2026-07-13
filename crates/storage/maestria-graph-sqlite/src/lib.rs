#![forbid(unsafe_code)]

//! SQLite-backed graph projection for Maestria.
//!
//! This crate stores domain relations in a rebuildable edge table. The domain
//! event log remains the source of truth; this adapter only serves `GraphIndex`
//! reads for projected graph edges.
//!
//! Runtime wiring owns projection updates; this adapter only persists and
//! serves rebuildable graph edges.

mod conversion;
mod migration;

use conversion::{read_relation, relation_endpoint_to_parts, relation_kind_to_str, to_port_error};
use migration::migrate;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use maestria_domain::{Relation, RelationEndpoint, RelationId};
use maestria_ports::{GraphIndex, PortError};
use rusqlite::{Connection, params};

/// SQLite-backed implementation of the graph relation projection.
pub struct SqliteGraphIndex {
    connection: Mutex<Connection>,
}

impl SqliteGraphIndex {
    /// Opens a SQLite database at `path` and applies the graph projection schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PortError> {
        let mut connection = Connection::open(path).map_err(to_port_error)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Creates an in-memory graph projection.
    pub fn in_memory() -> Result<Self, PortError> {
        let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Wraps an existing SQLite connection and applies the graph projection schema.
    pub fn from_connection(mut connection: Connection) -> Result<Self, PortError> {
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock_connection(&self) -> Result<MutexGuard<'_, Connection>, PortError> {
        self.connection.lock().map_err(|_| PortError::Internal {
            message: "graph sqlite connection lock poisoned".to_string(),
        })
    }
}

impl GraphIndex for SqliteGraphIndex {
    fn insert_relation(&self, relation: Relation) -> Result<(), PortError> {
        let (source_type, source_id) = relation_endpoint_to_parts(relation.source);
        let (target_type, target_id) = relation_endpoint_to_parts(relation.target);
        let evidence_id = relation.evidence_id.map(|id| id.value().to_string());
        let confidence_milli = i64::from(relation.confidence_milli);
        let connection = self.lock_connection()?;

        connection
            .execute(
                "INSERT INTO relations (
                     id,
                     source_type,
                     source_id,
                     kind,
                     target_type,
                     target_id,
                     evidence_id,
                     confidence_milli
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                     source_type = excluded.source_type,
                     source_id = excluded.source_id,
                     kind = excluded.kind,
                     target_type = excluded.target_type,
                     target_id = excluded.target_id,
                     evidence_id = excluded.evidence_id,
                     confidence_milli = excluded.confidence_milli",
                params![
                    relation.id.value().to_string(),
                    source_type,
                    source_id,
                    relation_kind_to_str(relation.kind),
                    target_type,
                    target_id,
                    evidence_id,
                    confidence_milli,
                ],
            )
            .map_err(to_port_error)?;
        Ok(())
    }

    fn get_relations_for(&self, endpoint: RelationEndpoint) -> Result<Vec<Relation>, PortError> {
        let (endpoint_type, endpoint_id) = relation_endpoint_to_parts(endpoint);
        let connection = self.lock_connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id,
                        source_type,
                        source_id,
                        kind,
                        target_type,
                        target_id,
                        evidence_id,
                        confidence_milli
                 FROM relations
                 WHERE (source_type = ?1 AND source_id = ?2)
                    OR (target_type = ?1 AND target_id = ?2)",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![endpoint_type, endpoint_id])
            .map_err(to_port_error)?;
        let mut relations = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            relations.push(read_relation(row)?);
        }
        relations.sort_by_key(|r| r.id);
        Ok(relations)
    }

    fn delete_relations(&self, relation_ids: &[RelationId]) -> Result<(), PortError> {
        if relation_ids.is_empty() {
            return Ok(());
        }
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        {
            let mut statement = transaction
                .prepare("DELETE FROM relations WHERE id = ?1")
                .map_err(to_port_error)?;
            for id in relation_ids {
                statement
                    .execute(params![id.value().to_string()])
                    .map_err(to_port_error)?;
            }
        }
        transaction.commit().map_err(to_port_error)?;
        Ok(())
    }

    fn clear(&self) -> Result<(), PortError> {
        let connection = self.lock_connection()?;
        connection
            .execute("DELETE FROM relations", params![])
            .map_err(to_port_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
