use maestria_domain::{ArtifactId, CardId, ChunkId};
use maestria_ports::{
    IndexedCard, IndexedChunk, IndexedLexicalCard, IndexedLexicalChunk, PortError,
};
use tantivy::{TantivyDocument, doc, schema::Value};

use super::{TantivyFullTextIndex, card_key, chunk_key};

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

    pub(crate) fn read_chunk(&self, document: &TantivyDocument) -> Result<IndexedChunk, PortError> {
        let artifact_id = document
            .get_first(self.fields.artifact_id)
            .and_then(|value| value.as_u64())
            .map(ArtifactId::new)
            .ok_or_else(|| PortError::Internal {
                message: "indexed chunk is missing artifact id".to_string(),
            })?;
        let chunk_id = document
            .get_first(self.fields.chunk_id)
            .and_then(|value| value.as_u64())
            .map(ChunkId::new)
            .ok_or_else(|| PortError::Internal {
                message: "indexed chunk is missing chunk id".to_string(),
            })?;
        let text = document
            .get_first(self.fields.text)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| PortError::Internal {
                message: "indexed chunk is missing text".to_string(),
            })?;

        Ok(IndexedChunk {
            artifact_id,
            chunk_id,
            text,
        })
    }

    pub(crate) fn read_lexical_chunk(
        &self,
        document: &TantivyDocument,
    ) -> Result<IndexedLexicalChunk, PortError> {
        let base = self.read_chunk(document)?;
        let path = document
            .get_first(self.fields.path)
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let filename = document
            .get_first(self.fields.filename)
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let symbol = document
            .get_first(self.fields.symbol)
            .and_then(|value| value.as_str())
            .map(str::to_string);
        Ok(IndexedLexicalChunk {
            artifact_id: base.artifact_id,
            chunk_id: base.chunk_id,
            text: base.text,
            path,
            filename,
            symbol,
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
            .ok_or_else(|| PortError::Internal {
                message: "indexed card is missing artifact id".to_string(),
            })?;
        let card_id = document
            .get_first(self.fields.card_id)
            .and_then(|value| value.as_u64())
            .map(CardId::new)
            .ok_or_else(|| PortError::Internal {
                message: "indexed card is missing card id".to_string(),
            })?;
        let title = document
            .get_first(self.fields.card_title)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| PortError::Internal {
                message: "indexed card is missing title".to_string(),
            })?;
        let body = document
            .get_first(self.fields.card_body)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| PortError::Internal {
                message: "indexed card is missing body".to_string(),
            })?;

        Ok(IndexedCard {
            artifact_id,
            card_id,
            title,
            body,
        })
    }

    pub(crate) fn read_lexical_card(
        &self,
        document: &TantivyDocument,
    ) -> Result<IndexedLexicalCard, PortError> {
        let base = self.read_card(document)?;
        let path = document
            .get_first(self.fields.card_path)
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let filename = document
            .get_first(self.fields.card_filename)
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let symbol = document
            .get_first(self.fields.card_symbol)
            .and_then(|value| value.as_str())
            .map(str::to_string);
        Ok(IndexedLexicalCard {
            artifact_id: base.artifact_id,
            card_id: base.card_id,
            title: base.title,
            body: base.body,
            path,
            filename,
            symbol,
        })
    }
}
