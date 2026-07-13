use maestria_ports::PortError;
use rusqlite::{Connection, OptionalExtension};

use crate::encoding::to_port_error;

pub(crate) const SCHEMA_VERSION: i64 = 2;
pub(crate) const SQLITE_VEC_BOOTSTRAP_SQL: &str =
    "CREATE VIRTUAL TABLE IF NOT EXISTS vec_docs USING vec0(chunk_id TEXT, embedding float[1536])";

pub(crate) fn migrate(connection: &mut Connection) -> Result<(), PortError> {
    let transaction = connection.transaction().map_err(to_port_error)?;

    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS vector_projection_schema (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 version INTEGER NOT NULL
             );",
        )
        .map_err(to_port_error)?;

    let version = transaction
        .query_row(
            "SELECT version FROM vector_projection_schema WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(to_port_error)?;

    if let Some(v) = version {
        if !(1..=SCHEMA_VERSION).contains(&v) {
            return Err(PortError::Internal {
                message: format!("unsupported vector projection schema version {v}"),
            });
        }
        if v == 1 {
            transaction
                .execute_batch(
                    "ALTER TABLE vector_embeddings ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
                     ALTER TABLE vector_embeddings ADD COLUMN model_version TEXT NOT NULL DEFAULT '';
                     UPDATE vector_projection_schema SET version = 2 WHERE id = 1;",
                )
                .map_err(to_port_error)?;
        }
    } else {
        transaction
            .execute_batch(
                "INSERT INTO vector_projection_schema (id, version) VALUES (1, 2);
                 CREATE TABLE IF NOT EXISTS vector_embeddings (
                     chunk_id INTEGER PRIMARY KEY NOT NULL,
                     dimension INTEGER NOT NULL,
                     embedding BLOB NOT NULL,
                     content_hash TEXT NOT NULL,
                     model_version TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_vector_embeddings_dimension
                     ON vector_embeddings(dimension);",
            )
            .map_err(to_port_error)?;
    }

    // verify the schema
    transaction
        .query_row(
            "SELECT chunk_id, dimension, embedding, content_hash, model_version FROM vector_embeddings LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(to_port_error)?;

    transaction.commit().map_err(to_port_error)?;

    attempt_sqlite_vec_bootstrap(connection);
    Ok(())
}

pub(crate) fn attempt_sqlite_vec_bootstrap(connection: &Connection) {
    let _ = connection.execute(SQLITE_VEC_BOOTSTRAP_SQL, []);
}

pub(crate) fn sqlite_vec_available(connection: &Connection) -> Result<bool, PortError> {
    let sql: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'vec_docs'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;

    let Some(sql) = sql else {
        return Ok(false);
    };

    let sql_upper = sql.to_uppercase();
    let normalized = sql_upper.split_whitespace().collect::<Vec<_>>();

    if normalized.len() >= 3
        && normalized[0] == "CREATE"
        && normalized[1] == "VIRTUAL"
        && normalized[2] == "TABLE"
    {
        let using_idx = normalized.iter().position(|&t| t == "USING");
        if let Some(module) = using_idx.and_then(|idx| normalized.get(idx + 1)) {
            let unquoted = module.replace(['\'', '"', '`'], "");
            if unquoted == "VEC0" || unquoted.starts_with("VEC0(") {
                return Ok(true);
            }
        }
    }

    Ok(false)
}
