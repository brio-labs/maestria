use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use maestria_domain::{ChunkId, SearchExecutionCompletion, SearchExecutionResource};
use maestria_ports::{
    BoundedSearch, IndexedEmbeddingKey, PortError, VectorEmbedding, VectorIndex, VectorSearchHit,
    VectorSearchQuery, execution::Meter,
};
use rusqlite::{Connection, params};

use crate::encoding::{
    PreparedEmbedding, cosine_similarity_bytes, i64_to_u64, to_port_error, u64_to_i64,
    usize_to_i64, validate_vector,
};
use crate::operations::{delete_stale_chunks, upsert_embeddings};
use crate::schema::migrate;

/// SQLite-backed implementation of the vector-search projection.
pub struct SqliteVectorIndex {
    pub(crate) connection: Mutex<Connection>,
}

impl SqliteVectorIndex {
    /// Opens a SQLite database at `path` and applies the vector projection schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PortError> {
        let mut connection = maestria_sqlite_support::open_connection(path)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Count persisted vector rows in the projection.
    ///
    /// Used by clients that report live embedding progress while a writer
    /// commits concurrently; the projection runs in WAL mode so the count
    /// reads a consistent committed snapshot.
    pub fn embedding_row_count(&self) -> Result<u64, PortError> {
        let connection = self.lock_connection()?;
        let count = connection
            .query_row("SELECT count(*) FROM vector_embeddings", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(to_port_error)?;
        i64_to_u64(count)
    }

    /// Creates an in-memory vector projection. Useful for adapter tests and callers
    /// that want an ephemeral projection.
    pub fn in_memory() -> Result<Self, PortError> {
        let mut connection = maestria_sqlite_support::open_in_memory_connection()?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock_connection(&self) -> Result<MutexGuard<'_, Connection>, PortError> {
        maestria_sqlite_support::lock_connection(&self.connection, "vector index lock poisoned")
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
        upsert_embeddings(&transaction, prepared)?;
        transaction.commit().map_err(to_port_error)
    }

    fn search_similar(
        &self,
        query: VectorSearchQuery,
    ) -> Result<BoundedSearch<VectorSearchHit>, PortError> {
        search_impl(self, query, &|_| Ok(true))
    }

    fn search_similar_filtered(
        &self,
        query: VectorSearchQuery,
        filter: &dyn Fn(ChunkId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<VectorSearchHit>, PortError> {
        search_impl(self, query, filter)
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

    fn rebuild(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), PortError> {
        let prepared = embeddings
            .into_iter()
            .map(PreparedEmbedding::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let mut expected_chunks = prepared
            .iter()
            .map(|embedding| u64_to_i64(embedding.chunk_id.value()))
            .collect::<Result<Vec<_>, _>>()?;
        expected_chunks.sort_unstable();

        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        upsert_embeddings(&transaction, prepared)?;
        delete_stale_chunks(&transaction, &expected_chunks)?;
        transaction.commit().map_err(to_port_error)
    }

    fn indexed_embedding_keys(&self) -> Result<Vec<IndexedEmbeddingKey>, PortError> {
        let connection = self.lock_connection()?;
        let mut statement = connection
            .prepare(
                "SELECT chunk_id, content_hash, generation_id, representation, fingerprint
                 FROM vector_embeddings",
            )
            .map_err(to_port_error)?;
        let rows = statement
            .query_map([], |row| {
                let stored_id: i64 = row.get(0)?;
                let chunk_id = u64::try_from(stored_id).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?;
                Ok(IndexedEmbeddingKey {
                    chunk_id: ChunkId::new(chunk_id),
                    content_hash: row.get(1)?,
                    generation_id: row.get(2)?,
                    representation: row.get(3)?,
                    fingerprint: row.get(4)?,
                })
            })
            .map_err(to_port_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(to_port_error)
    }

    fn reconcile_projection(
        &self,
        upserted: Vec<VectorEmbedding>,
        expected: &[ChunkId],
    ) -> Result<(), PortError> {
        let prepared = upserted
            .into_iter()
            .map(PreparedEmbedding::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let mut expected_chunks = expected
            .iter()
            .map(|chunk_id| u64_to_i64(chunk_id.value()))
            .collect::<Result<Vec<_>, _>>()?;
        expected_chunks.sort_unstable();

        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        upsert_embeddings(&transaction, prepared)?;
        delete_stale_chunks(&transaction, &expected_chunks)?;
        transaction.commit().map_err(to_port_error)
    }
}

fn collect_hits(
    rows: &mut rusqlite::Rows<'_>,
    query_vector: &[f32],
    query_norm_sqrt: f64,
    filter: &dyn Fn(ChunkId) -> Result<bool, PortError>,
    meter: &mut Meter,
) -> Result<(Vec<VectorSearchHit>, Option<SearchExecutionResource>), PortError> {
    let mut hits = Vec::new();
    let mut stopped = None;
    while let Some(row) = rows.next().map_err(to_port_error)? {
        if let Some(resource) = meter.candidate() {
            stopped = Some(resource);
            break;
        }
        let chunk_id = ChunkId::new(i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?);
        if !filter(chunk_id)? {
            continue;
        }
        let work = maestria_domain::saturating_u64(query_vector.len()).saturating_add(1);
        if let Some(resource) = meter.work(work) {
            stopped = Some(resource);
            break;
        }
        let bytes = row
            .get_ref(1)
            .map_err(to_port_error)?
            .as_blob()
            .map_err(|error| PortError::InternalContext {
                context: "read vector blob",
                source: error.to_string(),
            })?;
        let bytes_read = bytes.len() as u64;
        if let Some(resource) = meter.bytes(bytes_read) {
            stopped = Some(resource);
            break;
        }
        let score = cosine_similarity_bytes(query_vector, query_norm_sqrt, bytes)?;
        hits.push(VectorSearchHit { chunk_id, score });
    }
    Ok((hits, stopped))
}

/// Orders hits by descending score with chunk-id tiebreak (total order).
fn order_hits(left: &VectorSearchHit, right: &VectorSearchHit) -> std::cmp::Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.chunk_id.value().cmp(&right.chunk_id.value()))
}

fn search_impl(
    index: &SqliteVectorIndex,
    query: VectorSearchQuery,
    filter: &dyn Fn(ChunkId) -> Result<bool, PortError>,
) -> Result<BoundedSearch<VectorSearchHit>, PortError> {
    validate_vector(&query.vector, "query vector")?;
    if let Some(identity) = &query.identity
        && identity.fingerprint.dimensions as usize != query.vector.len()
    {
        return Err(PortError::InvalidInputContext {
            context: "query vector dimension mismatch",
            source: "vector and identity fingerprint dimensions differ".to_string(),
        });
    }
    if u64::from(query.limit) != query.execution_budget.max_results() {
        return Err(PortError::InvalidInputContext {
            context: "vector search result limit",
            source: "query limit and execution budget max_results must agree".to_string(),
        });
    }
    let mut meter = Meter::new(query.execution_budget);
    if query.limit == 0 {
        return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
    }
    let q_norm_sq: f64 = query.vector.iter().map(|&v| (v as f64) * (v as f64)).sum();
    if q_norm_sq == 0.0 {
        return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
    }
    let query_norm_sqrt = q_norm_sq.sqrt();
    let (gen_id, rep, fingerprint) = if let Some(identity) = &query.identity {
        (
            Some(identity.generation_id.value().to_string()),
            Some(identity.representation.0.clone()),
            Some(identity.fingerprint.encode()),
        )
    } else {
        (None, None, None)
    };
    let connection = index.lock_connection()?;
    let mut statement = connection
        .prepare(
            "SELECT chunk_id, embedding
                 FROM vector_embeddings
                 WHERE dimension = ?1
                   AND (?2 IS NULL OR provider_id = ?2)
                   AND (?3 IS NULL OR model = ?3)
                   AND (?4 IS NULL OR model_version = ?4)
                   AND (?5 IS NULL OR generation_id = ?5)
                   AND (?6 IS NULL OR representation = ?6)
                   AND (?7 IS NULL OR fingerprint = ?7)
                 ORDER BY chunk_id",
        )
        .map_err(to_port_error)?;
    let mut rows = statement
        .query(params![
            usize_to_i64(query.vector.len())?,
            query.provider_id.as_deref(),
            query.model.as_deref(),
            query.model_version.as_deref(),
            gen_id.as_deref(),
            rep.as_deref(),
            fingerprint.as_deref(),
        ])
        .map_err(to_port_error)?;
    let (mut hits, mut stopped) = collect_hits(
        &mut rows,
        &query.vector,
        query_norm_sqrt,
        filter,
        &mut meter,
    )?;
    let selected_limit =
        usize::try_from(query.limit).map_err(|_| PortError::InvalidInputContext {
            context: "vector search result limit",
            source: "result limit does not fit platform range".to_string(),
        })?;
    let result_exhausted = hits.len() > selected_limit;
    if selected_limit > 0 && hits.len() > selected_limit {
        hits.select_nth_unstable_by(selected_limit - 1, order_hits);
        hits.truncate(selected_limit);
    }
    hits.sort_by(order_hits);
    let selected = hits;
    if result_exhausted {
        stopped = Some(SearchExecutionResource::Results);
    }
    for _ in 0..selected.len() {
        if let Some(resource) = meter.result() {
            stopped = Some(resource);
            break;
        }
    }
    let completion = stopped.map_or(
        SearchExecutionCompletion::Complete,
        SearchExecutionCompletion::Exhausted,
    );
    Ok(meter.done(selected, completion))
}
