#![forbid(unsafe_code)]

//! SQLite-backed graph projection for Maestria.
//!
//! This crate stores domain relations in a rebuildable edge table. The domain
//! event log remains the source of truth; this adapter only serves `GraphIndex`
//! reads for projected graph edges.
//!
//! Note: Runtime integration and domain replay wiring are deferred to a
//! subsequent PR; they are not no-ops, but simply out of scope for this
//! projection layer update.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use maestria_domain::{
    ArtifactId, CardId, ClaimId, EvidenceId, MemoryId, Relation, RelationEndpoint, RelationId,
    RelationKind, TaskId,
};
use maestria_ports::{GraphIndex, PortError};
use rusqlite::{Connection, OptionalExtension, Row, params};

const SCHEMA_VERSION: i64 = 1;

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
}

fn migrate(connection: &mut Connection) -> Result<(), PortError> {
    let tx = connection.transaction().map_err(to_port_error)?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS graph_projection_schema (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             version INTEGER NOT NULL
         );",
    )
    .map_err(to_port_error)?;

    let current_version: Option<i64> = tx
        .query_row(
            "SELECT version FROM graph_projection_schema WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;

    match current_version {
        Some(v) if v == SCHEMA_VERSION => {}
        Some(v) => {
            return Err(PortError::Internal {
                message: format!("unsupported graph projection schema version {v}"),
            });
        }
        None => {
            tx.execute_batch(
                "CREATE TABLE relations (
                     id TEXT PRIMARY KEY,
                     source_type TEXT NOT NULL,
                     source_id TEXT NOT NULL,
                     kind TEXT NOT NULL,
                     target_type TEXT NOT NULL,
                     target_id TEXT NOT NULL,
                     evidence_id TEXT,
                     confidence_milli INTEGER NOT NULL
                 );
                 CREATE INDEX idx_relations_source
                     ON relations(source_type, source_id);
                 CREATE INDEX idx_relations_target
                     ON relations(target_type, target_id);
                 INSERT INTO graph_projection_schema (id, version) VALUES (1, 1);",
            )
            .map_err(to_port_error)?;
        }
    }

    let mut col_stmt = tx
        .prepare("PRAGMA table_info(relations)")
        .map_err(to_port_error)?;
    let mut cols_found = 0;
    let mut rows = col_stmt.query([]).map_err(to_port_error)?;
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let name: String = row.get(1).map_err(to_port_error)?;
        let ty: String = row.get(2).map_err(to_port_error)?;
        let ty = ty.to_uppercase();

        let valid = match name.as_str() {
            "id" | "source_type" | "source_id" | "kind" | "target_type" | "target_id"
            | "evidence_id" => ty == "TEXT",
            "confidence_milli" => ty == "INTEGER",
            _ => false,
        };
        if valid {
            cols_found += 1;
        }
    }
    if cols_found != 8 {
        return Err(PortError::Internal {
            message: "relations table is malformed or missing columns".to_string(),
        });
    }

    drop(rows);
    drop(col_stmt);

    let mut idx_stmt = tx
        .prepare("PRAGMA index_list(relations)")
        .map_err(to_port_error)?;
    let mut rows = idx_stmt.query([]).map_err(to_port_error)?;
    let mut has_source_idx = false;
    let mut has_target_idx = false;

    while let Some(row) = rows.next().map_err(to_port_error)? {
        let idx_name: String = row.get(1).map_err(to_port_error)?;
        if idx_name == "idx_relations_source" || idx_name == "idx_relations_target" {
            let mut info_stmt = tx
                .prepare(&format!("PRAGMA index_info({})", idx_name))
                .map_err(to_port_error)?;
            let mut info_rows = info_stmt.query([]).map_err(to_port_error)?;
            let mut cols = Vec::new();
            while let Some(info_row) = info_rows.next().map_err(to_port_error)? {
                let col_name: String = info_row.get(2).map_err(to_port_error)?;
                cols.push(col_name);
            }

            if idx_name == "idx_relations_source" && cols == ["source_type", "source_id"] {
                has_source_idx = true;
            } else if idx_name == "idx_relations_target" && cols == ["target_type", "target_id"] {
                has_target_idx = true;
            }
        }
    }

    if !has_source_idx || !has_target_idx {
        return Err(PortError::Internal {
            message: "relations table is missing required indexes".to_string(),
        });
    }

    drop(rows);
    drop(idx_stmt);

    tx.commit().map_err(to_port_error)?;
    Ok(())
}

fn read_relation(row: &Row<'_>) -> Result<Relation, PortError> {
    let id = row.get::<_, String>(0).map_err(to_port_error)?;
    let source_type = row.get::<_, String>(1).map_err(to_port_error)?;
    let source_id = row.get::<_, String>(2).map_err(to_port_error)?;
    let kind = row.get::<_, String>(3).map_err(to_port_error)?;
    let target_type = row.get::<_, String>(4).map_err(to_port_error)?;
    let target_id = row.get::<_, String>(5).map_err(to_port_error)?;
    let evidence_id = row.get::<_, Option<String>>(6).map_err(to_port_error)?;
    let confidence_milli = row.get::<_, i64>(7).map_err(to_port_error)?;

    Ok(Relation {
        id: parse_relation_id(&id)?,
        source: relation_endpoint_from_parts(&source_type, &source_id)?,
        kind: relation_kind_from_str(&kind)?,
        target: relation_endpoint_from_parts(&target_type, &target_id)?,
        evidence_id: evidence_id.as_deref().map(parse_evidence_id).transpose()?,
        confidence_milli: i64_to_u16(confidence_milli, "confidence_milli")?,
    })
}

fn relation_endpoint_to_parts(endpoint: RelationEndpoint) -> (&'static str, String) {
    match endpoint {
        RelationEndpoint::Artifact(id) => ("artifact", id.value().to_string()),
        RelationEndpoint::Claim(id) => ("claim", id.value().to_string()),
        RelationEndpoint::Task(id) => ("task", id.value().to_string()),
        RelationEndpoint::Memory(id) => ("memory", id.value().to_string()),
        RelationEndpoint::Card(id) => ("card", id.value().to_string()),
    }
}

fn relation_endpoint_from_parts(
    endpoint_type: &str,
    endpoint_id: &str,
) -> Result<RelationEndpoint, PortError> {
    match endpoint_type {
        "artifact" => Ok(RelationEndpoint::Artifact(ArtifactId::new(parse_u64(
            endpoint_id,
            "artifact id",
        )?))),
        "claim" => Ok(RelationEndpoint::Claim(ClaimId::new(parse_u64(
            endpoint_id,
            "claim id",
        )?))),
        "task" => Ok(RelationEndpoint::Task(TaskId::new(parse_u64(
            endpoint_id,
            "task id",
        )?))),
        "memory" => Ok(RelationEndpoint::Memory(MemoryId::new(parse_u64(
            endpoint_id,
            "memory id",
        )?))),
        "card" => Ok(RelationEndpoint::Card(CardId::new(parse_u64(
            endpoint_id,
            "card id",
        )?))),
        other => Err(PortError::Internal {
            message: format!("unknown relation endpoint type {other}"),
        }),
    }
}

fn relation_kind_to_str(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::Contains => "contains",
        RelationKind::Defines => "defines",
        RelationKind::Supports => "supports",
        RelationKind::Contradicts => "contradicts",
        RelationKind::UsedEvidence => "used_evidence",
        RelationKind::BasedOn => "based_on",
        RelationKind::DerivedFrom => "derived_from",
        RelationKind::AppliesTo => "applies_to",
        RelationKind::RelatedTo => "related_to",
    }
}

fn relation_kind_from_str(kind: &str) -> Result<RelationKind, PortError> {
    match kind {
        "contains" => Ok(RelationKind::Contains),
        "defines" => Ok(RelationKind::Defines),
        "supports" => Ok(RelationKind::Supports),
        "contradicts" => Ok(RelationKind::Contradicts),
        "used_evidence" => Ok(RelationKind::UsedEvidence),
        "based_on" => Ok(RelationKind::BasedOn),
        "derived_from" => Ok(RelationKind::DerivedFrom),
        "applies_to" => Ok(RelationKind::AppliesTo),
        "related_to" => Ok(RelationKind::RelatedTo),
        other => Err(PortError::Internal {
            message: format!("unknown relation kind {other}"),
        }),
    }
}

fn parse_relation_id(value: &str) -> Result<RelationId, PortError> {
    parse_u64(value, "relation id").map(RelationId::new)
}

fn parse_evidence_id(value: &str) -> Result<EvidenceId, PortError> {
    parse_u64(value, "evidence id").map(EvidenceId::new)
}

fn parse_u64(value: &str, label: &str) -> Result<u64, PortError> {
    value.parse::<u64>().map_err(|error| PortError::Internal {
        message: format!("stored {label} is invalid: {error}"),
    })
}

fn i64_to_u16(value: i64, label: &str) -> Result<u16, PortError> {
    u16::try_from(value).map_err(|_| PortError::Internal {
        message: format!("stored {label} {value} is outside u16 range"),
    })
}

fn to_port_error(error: rusqlite::Error) -> PortError {
    PortError::Internal {
        message: format!("sqlite graph projection error: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_relations_matching_either_endpoint() -> Result<(), PortError> {
        let index = SqliteGraphIndex::in_memory()?;
        let relation = Relation {
            id: RelationId::new(7),
            source: RelationEndpoint::Claim(ClaimId::new(20)),
            kind: RelationKind::Supports,
            target: RelationEndpoint::Artifact(ArtifactId::new(1)),
            evidence_id: Some(EvidenceId::new(40)),
            confidence_milli: 875,
        };

        index.insert_relation(relation.clone())?;

        assert_eq!(
            index.get_relations_for(RelationEndpoint::Claim(ClaimId::new(20)))?,
            vec![relation.clone()]
        );
        assert_eq!(
            index.get_relations_for(RelationEndpoint::Artifact(ArtifactId::new(1)))?,
            vec![relation]
        );
        assert!(
            index
                .get_relations_for(RelationEndpoint::Task(TaskId::new(50)))?
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn insert_relation_updates_existing_row() -> Result<(), PortError> {
        let index = SqliteGraphIndex::in_memory()?;
        let first = Relation {
            id: RelationId::new(7),
            source: RelationEndpoint::Claim(ClaimId::new(20)),
            kind: RelationKind::Supports,
            target: RelationEndpoint::Artifact(ArtifactId::new(1)),
            evidence_id: Some(EvidenceId::new(40)),
            confidence_milli: 875,
        };
        let updated = Relation {
            id: RelationId::new(7),
            source: RelationEndpoint::Memory(MemoryId::new(8)),
            kind: RelationKind::RelatedTo,
            target: RelationEndpoint::Card(CardId::new(9)),
            evidence_id: None,
            confidence_milli: 600,
        };

        index.insert_relation(first)?;
        index.insert_relation(updated.clone())?;

        assert_eq!(
            index.get_relations_for(RelationEndpoint::Memory(MemoryId::new(8)))?,
            vec![updated]
        );
        assert!(
            index
                .get_relations_for(RelationEndpoint::Claim(ClaimId::new(20)))?
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn returns_error_for_invalid_stored_relation() -> Result<(), PortError> {
        let index = SqliteGraphIndex::in_memory()?;

        let connection = index.lock_connection()?;
        connection.execute(
            "INSERT INTO relations (id, source_type, source_id, kind, target_type, target_id, confidence_milli)
             VALUES ('123', 'artifact', '1', 'unknown_kind', 'claim', '2', 1000)",
            [],
        ).map_err(to_port_error)?;
        drop(connection);

        let result = index.get_relations_for(RelationEndpoint::Artifact(ArtifactId::new(1)));
        assert!(matches!(
            result,
            Err(PortError::Internal { ref message }) if message.contains("unknown relation kind unknown_kind")
        ));
        Ok(())
    }

    #[test]
    fn orders_relations_by_numeric_id_not_lexical() -> Result<(), PortError> {
        let index = SqliteGraphIndex::in_memory()?;
        let relation_10 = Relation {
            id: RelationId::new(10),
            source: RelationEndpoint::Claim(ClaimId::new(1)),
            kind: RelationKind::Supports,
            target: RelationEndpoint::Artifact(ArtifactId::new(2)),
            evidence_id: None,
            confidence_milli: 1000,
        };
        let relation_2 = Relation {
            id: RelationId::new(2),
            source: RelationEndpoint::Claim(ClaimId::new(1)),
            kind: RelationKind::Supports,
            target: RelationEndpoint::Artifact(ArtifactId::new(3)),
            evidence_id: None,
            confidence_milli: 1000,
        };

        // Insert in arbitrary order
        index.insert_relation(relation_10.clone())?;
        index.insert_relation(relation_2.clone())?;

        let results = index.get_relations_for(RelationEndpoint::Claim(ClaimId::new(1)))?;

        // 2 should come before 10, whereas lexical sorting would put 10 before 2
        assert_eq!(results, vec![relation_2, relation_10]);
        Ok(())
    }

    #[test]
    fn satisfies_graph_index_contract() -> Result<(), PortError> {
        let index = SqliteGraphIndex::in_memory()?;
        maestria_ports::contract_tests::assert_graph_index_contract(&index);
        Ok(())
    }

    #[test]
    fn migration_is_idempotent() -> Result<(), PortError> {
        let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
        migrate(&mut conn)?; // Initial migration
        migrate(&mut conn)?; // Second should succeed without error (idempotent)
        Ok(())
    }

    #[test]
    fn rejects_unsupported_schema_version() -> Result<(), PortError> {
        let mut conn = Connection::open_in_memory().map_err(to_port_error)?;

        // Force an unsupported version manually
        conn.execute_batch(
            "CREATE TABLE graph_projection_schema (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 version INTEGER NOT NULL
             );
             INSERT INTO graph_projection_schema (id, version) VALUES (1, 9999);",
        )
        .map_err(to_port_error)?;

        let result = migrate(&mut conn);
        assert!(matches!(
            result,
            Err(PortError::Internal { ref message }) if message.contains("unsupported graph projection schema version 9999")
        ));
        Ok(())
    }

    #[test]
    fn rejects_version_claiming_missing_schema() -> Result<(), PortError> {
        let mut conn = Connection::open_in_memory().map_err(to_port_error)?;

        conn.execute_batch(
            "CREATE TABLE graph_projection_schema (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 version INTEGER NOT NULL
             );
             INSERT INTO graph_projection_schema (id, version) VALUES (1, 1);",
        )
        .map_err(to_port_error)?;

        let result = migrate(&mut conn);
        assert!(matches!(
            result,
            Err(PortError::Internal { ref message }) if message.contains("relations table is malformed or missing columns")
        ));
        Ok(())
    }
}
