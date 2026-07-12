use maestria_domain::{ArtifactId, ChunkId};
use maestria_ports::{IndexedChunk, PortError};
use tantivy::{
    Index, IndexReader, ReloadPolicy, TantivyDocument,
    collector::TopDocs,
    query::AllQuery,
    schema::{Schema, Value},
};

use super::{
    FIELD_ARTIFACT_ID, FIELD_CARD_ARTIFACT_ID, FIELD_CARD_BODY, FIELD_CARD_ID, FIELD_CARD_KEY,
    FIELD_CARD_TITLE, FIELD_CHUNK_ID, FIELD_TEXT, schema_field, to_port_error,
};

pub(super) fn schema_has_cards(schema: &Schema) -> bool {
    [
        FIELD_CARD_KEY,
        FIELD_CARD_ARTIFACT_ID,
        FIELD_CARD_ID,
        FIELD_CARD_TITLE,
        FIELD_CARD_BODY,
    ]
    .iter()
    .all(|name| schema.get_field(name).is_ok())
}

pub(super) fn legacy_chunks(index: &Index) -> Result<Vec<IndexedChunk>, PortError> {
    let schema = index.schema();
    let artifact_field = schema_field(&schema, FIELD_ARTIFACT_ID)?;
    let chunk_field = schema_field(&schema, FIELD_CHUNK_ID)?;
    let text_field = schema_field(&schema, FIELD_TEXT)?;
    let reader: IndexReader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()
        .map_err(to_port_error)?;
    let searcher = reader.searcher();
    let limit = searcher.num_docs() as usize;
    if limit == 0 {
        return Ok(Vec::new());
    }
    let documents = searcher
        .search(&AllQuery, &TopDocs::with_limit(limit).order_by_score())
        .map_err(to_port_error)?;
    let mut chunks = Vec::with_capacity(documents.len());
    for (_, address) in documents {
        let document = searcher
            .doc::<TantivyDocument>(address)
            .map_err(to_port_error)?;
        let artifact_id = document
            .get_first(artifact_field)
            .and_then(|value| value.as_u64())
            .map(ArtifactId::new)
            .ok_or_else(|| PortError::Internal {
                message: "legacy indexed chunk is missing artifact id".to_string(),
            })?;
        let chunk_id = document
            .get_first(chunk_field)
            .and_then(|value| value.as_u64())
            .map(ChunkId::new)
            .ok_or_else(|| PortError::Internal {
                message: "legacy indexed chunk is missing chunk id".to_string(),
            })?;
        let text = document
            .get_first(text_field)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| PortError::Internal {
                message: "legacy indexed chunk is missing text".to_string(),
            })?;
        chunks.push(IndexedChunk {
            artifact_id,
            chunk_id,
            text,
        });
    }
    Ok(chunks)
}
