use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use maestria_domain::{Relation, RelationId};
use maestria_ports::{GraphIndex, PortError};
use rusqlite::{Connection, params};

use crate::conversion::{
    read_relation, relation_endpoint_to_parts, relation_kind_to_str, to_port_error,
};
use crate::migration::migrate;

/// SQLite-backed implementation of the graph relation projection.
pub struct SqliteGraphIndex {
    connection: Mutex<Connection>,
}

impl SqliteGraphIndex {
    /// Opens a SQLite database at `path` and applies the graph projection schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PortError> {
        let mut connection = maestria_sqlite_support::open_connection(path)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Creates an in-memory graph projection.
    pub fn in_memory() -> Result<Self, PortError> {
        let mut connection = maestria_sqlite_support::open_in_memory_connection()?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub(crate) fn lock_connection(&self) -> Result<MutexGuard<'_, Connection>, PortError> {
        maestria_sqlite_support::lock_connection(
            &self.connection,
            "graph sqlite connection lock poisoned",
        )
    }
}

const INSERT_RELATION_SQL: &str = "INSERT INTO relations (
         id,
         source_type,
         source_id,
         kind,
         target_type,
         target_id,
         evidence_id,
         confidence_milli,
         security_json
     )
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
     ON CONFLICT(id) DO UPDATE SET
         source_type = excluded.source_type,
         source_id = excluded.source_id,
         kind = excluded.kind,
         target_type = excluded.target_type,
         target_id = excluded.target_id,
         evidence_id = excluded.evidence_id,
         confidence_milli = excluded.confidence_milli,
         security_json = excluded.security_json";

fn execute_insert_relation(
    statement: &mut rusqlite::CachedStatement<'_>,
    relation: Relation,
) -> Result<(), PortError> {
    let (source_type, source_id) = relation_endpoint_to_parts(relation.source);
    let (target_type, target_id) = relation_endpoint_to_parts(relation.target);
    let evidence_id = relation.evidence_id.map(|id| id.value().to_string());
    let confidence_milli = i64::from(relation.confidence_milli);

    statement
        .execute(params![
            relation.id.value().to_string(),
            source_type,
            source_id,
            relation_kind_to_str(relation.kind),
            target_type,
            target_id,
            evidence_id,
            confidence_milli,
            serde_json::to_string(&relation.security).map_err(|error| {
                PortError::InternalContext {
                    context: "serialize relation security",
                    source: error.to_string(),
                }
            })?,
        ])
        .map_err(to_port_error)?;
    Ok(())
}

fn insert_relation_with_connection(
    connection: &Connection,
    relation: Relation,
) -> Result<(), PortError> {
    let mut statement = connection
        .prepare_cached(INSERT_RELATION_SQL)
        .map_err(to_port_error)?;
    execute_insert_relation(&mut statement, relation)
}

fn rebuild_relations(
    connection: &mut Connection,
    relations: Vec<Relation>,
) -> Result<(), PortError> {
    let transaction = connection.transaction().map_err(to_port_error)?;
    transaction
        .execute("DELETE FROM relations", [])
        .map_err(to_port_error)?;
    {
        let mut statement = transaction
            .prepare_cached(INSERT_RELATION_SQL)
            .map_err(to_port_error)?;
        for relation in relations {
            execute_insert_relation(&mut statement, relation)?;
        }
    }
    transaction.commit().map_err(to_port_error)?;
    Ok(())
}

impl GraphIndex for SqliteGraphIndex {
    fn insert_relation(&self, relation: Relation) -> Result<(), PortError> {
        let connection = self.lock_connection()?;
        insert_relation_with_connection(&connection, relation)
    }

    fn rebuild(&self, relations: Vec<Relation>) -> Result<(), PortError> {
        let mut connection = self.lock_connection()?;
        rebuild_relations(&mut connection, relations)
    }

    fn get_relations_for(
        &self,
        query: maestria_ports::GraphRelationQuery,
    ) -> Result<maestria_ports::GraphRelationPage, PortError> {
        let (endpoint_type, endpoint_id) = relation_endpoint_to_parts(query.endpoint());
        let max_relations = query.max_relations();
        let fetch_limit = max_relations.saturating_add(1).min(i64::MAX as u64) as i64;
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
                        confidence_milli,
                        security_json
                 FROM relations
                 WHERE (source_type = ?1 AND source_id = ?2)
                    OR (target_type = ?1 AND target_id = ?2)
                 ORDER BY CAST(id AS INTEGER)
                 LIMIT ?3",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![endpoint_type, endpoint_id, fetch_limit])
            .map_err(to_port_error)?;
        let mut relations = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            relations.push(read_relation(row)?);
        }
        let complete = relations.len() <= maestria_domain::saturating_usize(max_relations);
        if !complete {
            relations.truncate(maestria_domain::saturating_usize(max_relations));
        }
        Ok(maestria_ports::GraphRelationPage {
            relations,
            complete,
        })
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
