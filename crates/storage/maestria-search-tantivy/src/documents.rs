use maestria_domain::{ArtifactId, CardId, ChunkId};
use maestria_ports::{
    IndexedCard, IndexedChunk, IndexedLexicalCard, IndexedLexicalChunk, PortError,
};
use tantivy::{DocAddress, Searcher, TantivyDocument, columnar::ColumnValues, doc, schema::Value};

use crate::{
    error::to_port_error,
    keys::{card_key, chunk_key},
    schema::{FIELD_ARTIFACT_ID, FIELD_CARD_ARTIFACT_ID, FIELD_CARD_ID, FIELD_CHUNK_ID},
    tantivy_index::TantivyFullTextIndex,
};
pub(crate) const INDEXED_IDENTITY_BYTES: u64 = 2 * std::mem::size_of::<u64>() as u64;

impl TantivyFullTextIndex {
    pub(crate) fn chunk_document(&self, chunk: &IndexedChunk) -> TantivyDocument {
        doc!(
            self.fields.key => chunk_key(chunk.artifact_id, chunk.chunk_id),
            self.fields.artifact_id => chunk.artifact_id.value(),
            self.fields.chunk_id => chunk.chunk_id.value(),
            self.fields.text => chunk.text.clone(),
        )
    }

    pub(crate) fn lexical_chunk_document(&self, chunk: &IndexedLexicalChunk) -> TantivyDocument {
        let mut doc = doc!(
            self.fields.key => chunk_key(chunk.artifact_id, chunk.chunk_id),
            self.fields.artifact_id => chunk.artifact_id.value(),
            self.fields.chunk_id => chunk.chunk_id.value(),
            self.fields.text => chunk.text.clone(),
        );
        if let Some(path) = &chunk.path {
            doc.add_text(self.fields.path, path);
        }
        if let Some(filename) = &chunk.filename {
            doc.add_text(self.fields.filename, filename);
        }
        if let Some(symbol) = &chunk.symbol {
            doc.add_text(self.fields.symbol, symbol);
        }
        doc
    }

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

    pub(crate) fn card_document(&self, card: &IndexedCard) -> TantivyDocument {
        doc!(
            self.fields.card_key => card_key(card.artifact_id, card.card_id),
            self.fields.card_artifact_id => card.artifact_id.value(),
            self.fields.card_id => card.card_id.value(),
            self.fields.card_title => card.title.clone(),
            self.fields.card_body => card.body.clone(),
        )
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

    pub(crate) fn lexical_card_document(&self, card: &IndexedLexicalCard) -> TantivyDocument {
        let mut doc = doc!(
            self.fields.card_key => card_key(card.artifact_id, card.card_id),
            self.fields.card_artifact_id => card.artifact_id.value(),
            self.fields.card_id => card.card_id.value(),
            self.fields.card_title => card.title.clone(),
            self.fields.card_body => card.body.clone(),
        );
        if let Some(path) = &card.path {
            doc.add_text(self.fields.card_path, path);
        }
        if let Some(filename) = &card.filename {
            doc.add_text(self.fields.card_filename, filename);
        }
        if let Some(symbol) = &card.symbol {
            doc.add_text(self.fields.card_symbol, symbol);
        }
        doc
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
