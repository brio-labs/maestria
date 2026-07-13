#![forbid(unsafe_code)]

//! SQLite-backed vector projection for Maestria.
//!
//! The table in this crate is a rebuildable projection: the domain event log and
//! chunk store remain the source of truth. The adapter attempts to create a
//! `sqlite-vec` virtual table when the extension is already available on the
//! supplied connection, and always maintains a portable BLOB-backed table used by
//! the `VectorIndex` implementation.

use maestria_domain::ChunkId;
use maestria_ports::{PortError, VectorEmbedding, VectorIndex, VectorSearchHit, VectorSearchQuery};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

mod encoding;
mod schema;
pub(crate) use encoding::to_port_error;
use encoding::{
    PreparedEmbedding, cosine_similarity, decode_vector, i64_to_u64, u64_to_i64, usize_to_i64,
    validate_vector,
};

pub(crate) use schema::{migrate, sqlite_vec_available};
#[cfg(test)]
mod tests;

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
                let dimension = usize_to_i64(embedding.dimension)?;
                let unchanged = transaction
                    .query_row(
                        "SELECT dimension, embedding, content_hash, model_version
                         FROM vector_embeddings WHERE chunk_id = ?1",
                        params![u64_to_i64(embedding.chunk_id.value())?],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, Vec<u8>>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(to_port_error)?
                    .is_some_and(|(stored_dimension, bytes, content_hash, model_version)| {
                        stored_dimension == dimension
                            && bytes == embedding.bytes
                            && content_hash == embedding.content_hash
                            && model_version == embedding.model_version
                    });
                if unchanged {
                    continue;
                }
                statement
                    .execute(params![
                        u64_to_i64(embedding.chunk_id.value())?,
                        dimension,
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
                 WHERE dimension = ?1
                   AND (?2 IS NULL OR model_version = ?2)",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![
                usize_to_i64(query_dimension)?,
                query.model_version.as_deref(),
            ])
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

    fn delete_chunks(&self, chunk_ids: &[ChunkId]) -> Result<(), PortError> {
        if chunk_ids.is_empty() {
            return Ok(());
        }
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        {
            let mut statement = transaction
                .prepare("DELETE FROM vector_embeddings WHERE chunk_id = ?1")
                .map_err(to_port_error)?;
            for &chunk_id in chunk_ids {
                statement
                    .execute(params![u64_to_i64(chunk_id.value())?])
                    .map_err(to_port_error)?;
            }
        }
        transaction.commit().map_err(to_port_error)
    }

    fn clear(&self) -> Result<(), PortError> {
        let connection = self.lock_connection()?;
        connection
            .execute("DELETE FROM vector_embeddings", [])
            .map_err(to_port_error)?;
        Ok(())
    }
}
