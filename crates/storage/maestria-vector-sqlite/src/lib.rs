#![forbid(unsafe_code)]

//! SQLite-backed vector projection for Maestria.
//!
//! The table in this crate is a rebuildable projection: the domain event log and
//! chunk store remain the source of truth. The adapter attempts to create a
//! `sqlite-vec` virtual table when the extension is already available on the
//! supplied connection, and always maintains a portable BLOB-backed table used by
//! the `VectorIndex` implementation.

use std::{
    mem::size_of,
    path::Path,
    sync::{Mutex, MutexGuard},
};

use maestria_domain::ChunkId;
use maestria_ports::{PortError, VectorEmbedding, VectorIndex, VectorSearchHit, VectorSearchQuery};
use rusqlite::{Connection, OptionalExtension, params};

const SCHEMA_VERSION: i64 = 2;
const F32_BYTES: usize = size_of::<f32>();
const SQLITE_VEC_BOOTSTRAP_SQL: &str =
    "CREATE VIRTUAL TABLE IF NOT EXISTS vec_docs USING vec0(chunk_id TEXT, embedding float[1536])";

/// SQLite-backed implementation of the vector-search projection.
pub struct SqliteVectorIndex {
    connection: Mutex<Connection>,
}

impl SqliteVectorIndex {
    /// Opens a SQLite database at `path` and applies the vector projection schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PortError> {
        let mut connection = Connection::open(path).map_err(to_port_error)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Creates an in-memory vector projection. Useful for adapter tests and callers
    /// that want an ephemeral projection.
    pub fn in_memory() -> Result<Self, PortError> {
        let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Wraps an existing SQLite connection and applies the vector projection schema.
    pub fn from_connection(mut connection: Connection) -> Result<Self, PortError> {
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Returns true when the optional `sqlite-vec` virtual table could be created.
    pub fn sqlite_vec_available(&self) -> Result<bool, PortError> {
        let connection = self.lock_connection()?;
        sqlite_vec_available(&connection)
    }

    fn lock_connection(&self) -> Result<MutexGuard<'_, Connection>, PortError> {
        self.connection.lock().map_err(|_| PortError::Internal {
            message: "vector index lock poisoned".to_string(),
        })
    }
}

impl VectorIndex for SqliteVectorIndex {
    fn index_embeddings(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), PortError> {
        let prepared = embeddings
            .into_iter()
            .map(PreparedEmbedding::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        if prepared.is_empty() {
            return Ok(());
        }

        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO vector_embeddings (chunk_id, dimension, embedding, content_hash, model_version)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(chunk_id) DO UPDATE SET
                         dimension = excluded.dimension,
                         embedding = excluded.embedding,
                         content_hash = excluded.content_hash,
                         model_version = excluded.model_version",
                )
                .map_err(to_port_error)?;

            for embedding in prepared {
                statement
                    .execute(params![
                        u64_to_i64(embedding.chunk_id.value())?,
                        usize_to_i64(embedding.dimension)?,
                        embedding.bytes,
                        embedding.content_hash,
                        embedding.model_version,
                    ])
                    .map_err(to_port_error)?;
            }
        }
        transaction.commit().map_err(to_port_error)
    }

    fn search_similar(&self, query: VectorSearchQuery) -> Result<Vec<VectorSearchHit>, PortError> {
        validate_vector(&query.vector, "query vector")?;
        if query.limit == 0 {
            return Ok(Vec::new());
        }

        let q_norm_sq: f64 = query.vector.iter().map(|&v| (v as f64) * (v as f64)).sum();
        if q_norm_sq == 0.0 {
            return Ok(Vec::new());
        }

        let query_dimension = query.vector.len();
        let connection = self.lock_connection()?;
        let mut statement = connection
            .prepare(
                "SELECT chunk_id, embedding
                 FROM vector_embeddings
                 WHERE dimension = ?1",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![usize_to_i64(query_dimension)?])
            .map_err(to_port_error)?;

        let mut hits = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            let chunk_id = i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?;
            let bytes = row.get::<_, Vec<u8>>(1).map_err(to_port_error)?;
            let vector = decode_vector(&bytes)?;
            let score = cosine_similarity(&query.vector, &vector)?;
            hits.push(VectorSearchHit {
                chunk_id: ChunkId::new(chunk_id),
                score,
            });
        }

        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.chunk_id.value().cmp(&right.chunk_id.value()))
        });
        hits.truncate(query.limit as usize);
        Ok(hits)
    }
}

struct PreparedEmbedding {
    chunk_id: ChunkId,
    dimension: usize,
    bytes: Vec<u8>,
    content_hash: String,
    model_version: String,
}

impl TryFrom<VectorEmbedding> for PreparedEmbedding {
    type Error = PortError;

    fn try_from(embedding: VectorEmbedding) -> Result<Self, Self::Error> {
        validate_vector(&embedding.vector, "embedding vector")?;
        let dimension = embedding.vector.len();
        let bytes = encode_vector(&embedding.vector)?;
        Ok(Self {
            chunk_id: embedding.chunk_id,
            dimension,
            bytes,
            content_hash: embedding.provenance.content_hash,
            model_version: embedding.provenance.model_version,
        })
    }
}

fn migrate(connection: &mut Connection) -> Result<(), PortError> {
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

fn attempt_sqlite_vec_bootstrap(connection: &Connection) {
    let _ = connection.execute(SQLITE_VEC_BOOTSTRAP_SQL, []);
}

fn sqlite_vec_available(connection: &Connection) -> Result<bool, PortError> {
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

fn validate_vector(vector: &[f32], label: &str) -> Result<(), PortError> {
    if vector.is_empty() {
        return Err(PortError::InvalidInput {
            message: format!("{label} must not be empty"),
        });
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(PortError::InvalidInput {
            message: format!("{label} must contain only finite values"),
        });
    }
    Ok(())
}

fn encode_vector(vector: &[f32]) -> Result<Vec<u8>, PortError> {
    let capacity = vector
        .len()
        .checked_mul(F32_BYTES)
        .ok_or_else(|| PortError::InvalidInput {
            message: "embedding vector is too large".to_string(),
        })?;
    let mut bytes = Vec::with_capacity(capacity);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>, PortError> {
    if !bytes.len().is_multiple_of(F32_BYTES) {
        return Err(PortError::Internal {
            message: "stored vector blob has invalid length".to_string(),
        });
    }

    let mut vector = Vec::with_capacity(bytes.len() / F32_BYTES);
    for chunk in bytes.chunks_exact(F32_BYTES) {
        let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if !value.is_finite() {
            return Err(PortError::Internal {
                message: "stored vector blob contains non-finite value".to_string(),
            });
        }
        vector.push(value);
    }
    Ok(vector)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> Result<f32, PortError> {
    if left.len() != right.len() {
        return Err(PortError::Internal {
            message: "stored vector dimension does not match query vector".to_string(),
        });
    }

    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (left_value, right_value) in left.iter().zip(right.iter()) {
        let l = *left_value as f64;
        let r = *right_value as f64;
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        return Ok(0.0);
    }

    let score = (dot / (left_norm.sqrt() * right_norm.sqrt())) as f32;
    Ok(if score.is_finite() { score } else { 0.0 })
}

fn u64_to_i64(value: u64) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| PortError::InvalidInput {
        message: format!("id {value} exceeds sqlite integer range"),
    })
}

fn i64_to_u64(value: i64) -> Result<u64, PortError> {
    u64::try_from(value).map_err(|_| PortError::Internal {
        message: format!("stored id {value} is negative"),
    })
}

fn usize_to_i64(value: usize) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| PortError::InvalidInput {
        message: format!("dimension {value} exceeds sqlite integer range"),
    })
}

fn to_port_error(error: rusqlite::Error) -> PortError {
    PortError::Internal {
        message: format!("sqlite vector projection error: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use maestria_domain::ChunkId;
    use maestria_ports::{
        EmbeddingProvenance, PortError, VectorEmbedding, VectorIndex, VectorSearchQuery,
        contract_tests::assert_vector_index_contract,
    };
    use rusqlite::Connection;

    use super::{SCHEMA_VERSION, SqliteVectorIndex, to_port_error};

    #[test]
    fn satisfies_shared_vector_index_contract() -> Result<(), PortError> {
        let index = SqliteVectorIndex::in_memory()?;
        assert_vector_index_contract(&index);
        Ok(())
    }

    #[test]
    fn round_trips_provenance() -> Result<(), PortError> {
        let index = SqliteVectorIndex::in_memory()?;
        let provenance = EmbeddingProvenance {
            content_hash: "hash_abcd".into(),
            model_version: "model_v1".into(),
        };

        index.index_embeddings(vec![VectorEmbedding {
            chunk_id: ChunkId::new(42),
            vector: vec![1.0, 0.5, 0.25],
            provenance: provenance.clone(),
        }])?;

        // Direct query to verify provenance storage, since the contract
        let connection = index.connection.lock().map_err(|_| PortError::Internal {
            message: "vector index lock poisoned".to_string(),
        })?;
        let mut stmt = connection
            .prepare(
                "SELECT content_hash, model_version FROM vector_embeddings WHERE chunk_id = 42",
            )
            .map_err(to_port_error)?;
        let (hash, version): (String, String) = stmt
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(to_port_error)?;

        assert_eq!(hash, provenance.content_hash);
        assert_eq!(version, provenance.model_version);
        Ok(())
    }

    #[test]
    fn rejects_unsupported_schema_version() -> Result<(), PortError> {
        let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
        conn.execute_batch(
            "CREATE TABLE vector_projection_schema (id INTEGER PRIMARY KEY, version INTEGER);
             INSERT INTO vector_projection_schema (id, version) VALUES (1, 999);",
        )
        .map_err(to_port_error)?;

        match super::migrate(&mut conn) {
            Err(PortError::Internal { message }) => {
                assert!(message.contains("unsupported vector projection schema version 999"));
            }
            Err(_) => {
                return Err(PortError::Internal {
                    message: "Expected unsupported version error, got different error".to_string(),
                });
            }
            Ok(_) => {
                return Err(PortError::Internal {
                    message: "Expected error but got Ok".to_string(),
                });
            }
        }
        Ok(())
    }
    #[test]
    fn rejects_zero_schema_version() -> Result<(), PortError> {
        let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
        conn.execute_batch(
            "CREATE TABLE vector_projection_schema (id INTEGER PRIMARY KEY, version INTEGER);
             INSERT INTO vector_projection_schema (id, version) VALUES (1, 0);",
        )
        .map_err(to_port_error)?;

        match super::migrate(&mut conn) {
            Err(PortError::Internal { message }) => {
                assert!(message.contains("unsupported vector projection schema version 0"));
            }
            Err(_) => {
                return Err(PortError::Internal {
                    message: "Expected unsupported version error, got different error".to_string(),
                });
            }
            Ok(_) => {
                return Err(PortError::Internal {
                    message: "Expected error but got Ok".to_string(),
                });
            }
        }
        Ok(())
    }

    #[test]
    fn migrates_version_1_schema_to_current() -> Result<(), PortError> {
        let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
        conn.execute_batch(
            "CREATE TABLE vector_projection_schema (id INTEGER PRIMARY KEY, version INTEGER);
             INSERT INTO vector_projection_schema (id, version) VALUES (1, 1);
             CREATE TABLE vector_embeddings (
                 chunk_id INTEGER PRIMARY KEY NOT NULL,
                 dimension INTEGER NOT NULL,
                 embedding BLOB NOT NULL
             );",
        )
        .map_err(to_port_error)?;

        super::migrate(&mut conn)?;

        let v: i64 = conn
            .query_row(
                "SELECT version FROM vector_projection_schema WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .map_err(to_port_error)?;
        assert_eq!(v, SCHEMA_VERSION);

        // Verify new columns exist by doing a dummy insert
        conn.execute("INSERT INTO vector_embeddings (chunk_id, dimension, embedding, content_hash, model_version) VALUES (1, 1, X'00', 'a', 'b')", []).map_err(to_port_error)?;
        Ok(())
    }

    #[test]
    fn sqlite_vec_detection_verifies_virtual_table() -> Result<(), PortError> {
        let conn = Connection::open_in_memory().map_err(to_port_error)?;
        // Create a regular table named vec_docs with spoofed comment
        conn.execute("CREATE TABLE vec_docs (id INTEGER /* USING VEC0 */)", [])
            .map_err(to_port_error)?;

        assert!(!super::sqlite_vec_available(&conn)?);

        Ok(())
    }

    #[test]
    fn prevents_nan_scores_from_overflow() -> Result<(), PortError> {
        let index = SqliteVectorIndex::in_memory()?;
        let prov = EmbeddingProvenance {
            content_hash: "hash".into(),
            model_version: "v1".into(),
        };

        // Vectors that might cause f32 overflow when accumulating sum of squares
        // e.g. a vector with values near sqrt(f32::MAX) ~= 1.8e19
        let huge_val = 1.0e19_f32;
        index.index_embeddings(vec![VectorEmbedding {
            chunk_id: ChunkId::new(1),
            vector: vec![huge_val, huge_val],
            provenance: prov,
        }])?;

        let hits = index.search_similar(VectorSearchQuery {
            vector: vec![huge_val, huge_val],
            limit: 1,
        })?;

        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].score.is_finite(),
            "Score should be finite despite huge values"
        );
        assert_eq!(hits[0].score, 1.0); // Exact match is 1.0
        Ok(())
    }
}
