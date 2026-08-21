use maestria_ports::PortError;
use rusqlite::{Transaction, params};

use crate::encoding::{PreparedEmbedding, to_port_error, u64_to_i64};

pub(crate) fn upsert_embeddings(
    transaction: &Transaction<'_>,
    prepared: Vec<PreparedEmbedding>,
) -> Result<(), PortError> {
    let mut statement = transaction
        .prepare(
            "INSERT INTO vector_embeddings
                 (chunk_id, dimension, embedding, content_hash, provider_id, model, model_version, generation_id, representation, fingerprint, disclosure_remote, retention_policy)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(chunk_id) DO UPDATE SET
                 dimension = excluded.dimension,
                 embedding = excluded.embedding,
                 content_hash = excluded.content_hash,
                 provider_id = excluded.provider_id,
                 model = excluded.model,
                 model_version = excluded.model_version,
                 generation_id = excluded.generation_id,
                 representation = excluded.representation,
                 fingerprint = excluded.fingerprint,
                 disclosure_remote = excluded.disclosure_remote,
                 retention_policy = excluded.retention_policy
             WHERE vector_embeddings.dimension != excluded.dimension
                OR vector_embeddings.embedding != excluded.embedding
                OR vector_embeddings.content_hash != excluded.content_hash
                OR vector_embeddings.provider_id != excluded.provider_id
                OR vector_embeddings.model != excluded.model
                OR vector_embeddings.model_version != excluded.model_version
                OR vector_embeddings.generation_id != excluded.generation_id
                OR vector_embeddings.representation != excluded.representation
                OR vector_embeddings.fingerprint != excluded.fingerprint
                OR vector_embeddings.disclosure_remote != excluded.disclosure_remote
                OR vector_embeddings.retention_policy IS NOT excluded.retention_policy",
        )
        .map_err(to_port_error)?;

    for embedding in prepared {
        let dimension = crate::encoding::usize_to_i64(embedding.dimension)?;
        statement
            .execute(params![
                u64_to_i64(embedding.chunk_id.value())?,
                dimension,
                embedding.bytes,
                embedding.content_hash,
                embedding.provider_id,
                embedding.model,
                embedding.model_version,
                embedding.generation_id,
                embedding.representation,
                embedding.fingerprint,
                i64::from(u8::from(embedding.disclosure_remote)),
                embedding.retention_policy,
            ])
            .map_err(to_port_error)?;
    }
    Ok(())
}

pub(crate) fn delete_stale_chunks(
    transaction: &Transaction<'_>,
    expected_chunks: &[i64],
) -> Result<(), PortError> {
    let mut stale_chunks = Vec::new();
    {
        let mut query = transaction
            .prepare("SELECT chunk_id FROM vector_embeddings")
            .map_err(to_port_error)?;
        let mut rows = query.query([]).map_err(to_port_error)?;
        while let Some(row) = rows.next().map_err(to_port_error)? {
            let id: i64 = row.get(0).map_err(to_port_error)?;
            if expected_chunks.binary_search(&id).is_err() {
                stale_chunks.push(id);
            }
        }
    }

    let mut statement = transaction
        .prepare("DELETE FROM vector_embeddings WHERE chunk_id = ?")
        .map_err(to_port_error)?;
    for id in stale_chunks {
        statement.execute(params![id]).map_err(to_port_error)?;
    }
    Ok(())
}
