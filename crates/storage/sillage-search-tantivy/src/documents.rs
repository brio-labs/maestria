use sillage_domain::{ArtifactId, CardId, ChunkId};
use sillage_ports::{IndexedCard, IndexedChunk, PortError};
use tantivy::{DocAddress, Searcher, TantivyDocument, columnar::ColumnValues, schema::Value};

use crate::{
    error::to_port_error,
    schema::{FIELD_ARTIFACT_ID, FIELD_CARD_ARTIFACT_ID, FIELD_CARD_ID, FIELD_CHUNK_ID},
    tantivy_index::TantivyFullTextIndex,
};
pub(crate) const INDEXED_IDENTITY_BYTES: u64 = 2 * std::mem::size_of::<u64>() as u64;

impl TantivyFullTextIndex {
    pub(crate) fn read_chunk_identity_at(
        &self,
        searcher: &Searcher,
        address: DocAddress,
    ) -> Result<(ArtifactId, ChunkId), PortError> {
        let segment = searcher.segment_reader(address.segment_ord);
        let artifact_id = segment
            .fast_fields()
            .u64(FIELD_ARTIFACT_ID)
            .map_err(to_port_error)?
            .first_or_default_col(0)
            .get_val(address.doc_id);
        let chunk_id = segment
            .fast_fields()
            .u64(FIELD_CHUNK_ID)
            .map_err(to_port_error)?
            .first_or_default_col(0)
            .get_val(address.doc_id);
        Ok((ArtifactId::new(artifact_id), ChunkId::new(chunk_id)))
    }

    pub(crate) fn read_chunk(&self, document: &TantivyDocument) -> Result<IndexedChunk, PortError> {
        let artifact_id = document
            .get_first(self.fields.artifact_id)
            .and_then(|value| value.as_u64())
            .map(ArtifactId::new)
            .ok_or_else(|| {
                PortError::internal(
                    "decode indexed chunk artifact id",
                    "indexed chunk is missing artifact id",
                )
            })?;
        let chunk_id = document
            .get_first(self.fields.chunk_id)
            .and_then(|value| value.as_u64())
            .map(ChunkId::new)
            .ok_or_else(|| {
                PortError::internal(
                    "decode indexed chunk id",
                    "indexed chunk is missing chunk id",
                )
            })?;
        let text = document
            .get_first(self.fields.text)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                PortError::internal("decode indexed chunk text", "indexed chunk is missing text")
            })?;

        Ok(IndexedChunk {
            artifact_id,
            chunk_id,
            text,
        })
    }

    pub(crate) fn read_card_identity_at(
        &self,
        searcher: &Searcher,
        address: DocAddress,
    ) -> Result<(ArtifactId, CardId), PortError> {
        let segment = searcher.segment_reader(address.segment_ord);
        let artifact_id = segment
            .fast_fields()
            .u64(FIELD_CARD_ARTIFACT_ID)
            .map_err(to_port_error)?
            .first_or_default_col(0)
            .get_val(address.doc_id);
        let card_id = segment
            .fast_fields()
            .u64(FIELD_CARD_ID)
            .map_err(to_port_error)?
            .first_or_default_col(0)
            .get_val(address.doc_id);
        Ok((ArtifactId::new(artifact_id), CardId::new(card_id)))
    }

    pub(crate) fn read_card(&self, document: &TantivyDocument) -> Result<IndexedCard, PortError> {
        let artifact_id = document
            .get_first(self.fields.card_artifact_id)
            .and_then(|value| value.as_u64())
            .map(ArtifactId::new)
            .ok_or_else(|| {
                PortError::internal(
                    "decode indexed card artifact id",
                    "indexed card is missing artifact id",
                )
            })?;
        let card_id = document
            .get_first(self.fields.card_id)
            .and_then(|value| value.as_u64())
            .map(CardId::new)
            .ok_or_else(|| {
                PortError::internal("decode indexed card id", "indexed card is missing card id")
            })?;
        let title = document
            .get_first(self.fields.card_title)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                PortError::internal("decode indexed card title", "indexed card is missing title")
            })?;
        let body = document
            .get_first(self.fields.card_body)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                PortError::internal("decode indexed card body", "indexed card is missing body")
            })?;

        Ok(IndexedCard {
            artifact_id,
            card_id,
            title,
            body,
        })
    }
}
