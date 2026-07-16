#![forbid(unsafe_code)]

//! Tantivy-backed full-text search projection for Maestria.
//!
//! This crate stores only rebuildable indexed chunks. Artifact metadata and blob
//! contents remain owned by their source repositories.

mod lexical_helpers;
mod lexical_operations;
mod migration;
mod operations;
mod schema;
mod search_helpers;
use maestria_governance::scan_secrets;
use migration::{legacy_chunks, schema_has_cards};
use schema::{load_fields, schema, supports_filtered_queries};
use std::{fs, path::Path, sync::Mutex};

use maestria_domain::{ArtifactId, CardId, ChunkId};
use maestria_ports::{FullTextIndex, IndexedCard, IndexedChunk, PortError};
use maestria_ports::{IndexedLexicalCard, IndexedLexicalChunk};
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, doc,
    schema::{Field, Schema, Value},
};

const WRITER_MEMORY_BUDGET_BYTES: usize = 50_000_000;
const FIELD_KEY: &str = "chunk_key";
const FIELD_ARTIFACT_ID: &str = "artifact_id";
const FIELD_CHUNK_ID: &str = "chunk_id";
const FIELD_TEXT: &str = "text";
const FIELD_CARD_KEY: &str = "card_key";
const FIELD_CARD_ARTIFACT_ID: &str = "card_artifact_id";
const FIELD_CARD_ID: &str = "card_id";
const FIELD_CARD_TITLE: &str = "card_title";
const FIELD_CARD_BODY: &str = "card_body";
const FIELD_PATH: &str = "path";
const FIELD_FILENAME: &str = "filename";
const FIELD_SYMBOL: &str = "symbol";
const FIELD_CARD_PATH: &str = "card_path";
const FIELD_CARD_FILENAME: &str = "card_filename";
const FIELD_CARD_SYMBOL: &str = "card_symbol";

/// Tantivy implementation of the [`FullTextIndex`] projection port.
pub struct TantivyFullTextIndex {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    fields: IndexFields,
    card_rebuild_required: Mutex<bool>,
    card_rebuild_marker: Option<std::path::PathBuf>,
}

struct IndexFields {
    key: Field,
    artifact_id: Field,
    chunk_id: Field,
    text: Field,
    card_key: Field,
    card_artifact_id: Field,
    card_id: Field,
    card_title: Field,
    card_body: Field,
    path: Field,
    filename: Field,
    symbol: Field,
    card_path: Field,
    card_filename: Field,
    card_symbol: Field,
}

impl TantivyFullTextIndex {
    pub fn in_memory() -> Result<Self, PortError> {
        Self::from_index(Index::create_in_ram(schema()), false, None)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, PortError> {
        let path = path.as_ref();
        fs::create_dir_all(path).map_err(to_io_port_error)?;
        let temp_path = path.with_extension("migrating");
        let backup_path = path.with_extension("legacy");
        if !path.join("meta.json").exists() {
            if temp_path.join("meta.json").exists() {
                fs::remove_dir_all(path).map_err(to_io_port_error)?;
                fs::rename(&temp_path, path).map_err(to_io_port_error)?;
            } else if backup_path.join("meta.json").exists() {
                fs::remove_dir_all(path).map_err(to_io_port_error)?;
                fs::rename(&backup_path, path).map_err(to_io_port_error)?;
            }
        }
        let marker = path.join(".cards-rebuild");
        if path.join("meta.json").exists() {
            let existing = Index::open_in_dir(path).map_err(to_port_error)?;
            if schema_has_cards(&existing.schema())
                && supports_filtered_queries(&existing.schema())
                && migration::schema_has_lexical(&existing.schema())
            {
                let required = marker.exists();
                return Self::from_index(existing, required, Some(marker));
            }
            let chunks = legacy_chunks(&existing)?
                .into_iter()
                .filter(|chunk| scan_secrets(&chunk.text).is_clean())
                .collect();
            drop(existing);
            let temp_path = path.with_extension("migrating");
            if temp_path.exists() {
                fs::remove_dir_all(&temp_path).map_err(to_io_port_error)?;
            }
            fs::create_dir_all(&temp_path).map_err(to_io_port_error)?;
            let temp_marker = temp_path.join(".cards-rebuild");
            fs::write(&temp_marker, b"pending").map_err(to_io_port_error)?;
            let rebuilt = Index::create_in_dir(&temp_path, schema()).map_err(to_port_error)?;
            let projection = Self::from_index(rebuilt, true, Some(temp_marker))?;
            projection.index_chunks(chunks)?;
            drop(projection);

            let backup_path = path.with_extension("legacy");
            if backup_path.exists() {
                fs::remove_dir_all(&backup_path).map_err(to_io_port_error)?;
            }
            fs::rename(path, &backup_path).map_err(to_io_port_error)?;
            if let Err(error) = fs::rename(&temp_path, path) {
                let _ = fs::rename(&backup_path, path);
                return Err(to_io_port_error(error));
            }
            let migrated = Index::open_in_dir(path).map_err(to_port_error)?;
            let projection = Self::from_index(migrated, true, Some(marker));
            let _ = fs::remove_dir_all(&backup_path);
            return projection;
        }

        let index = Index::create_in_dir(path, schema()).map_err(to_port_error)?;
        let required = marker.exists();
        Self::from_index(index, required, Some(marker))
    }

    fn from_index(
        index: Index,
        card_rebuild_required: bool,
        card_rebuild_marker: Option<std::path::PathBuf>,
    ) -> Result<Self, PortError> {
        let fields = load_fields(index.schema())?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(to_port_error)?;
        let writer = index
            .writer_with_num_threads(1, WRITER_MEMORY_BUDGET_BYTES)
            .map_err(to_port_error)?;

        Ok(Self {
            index,
            reader,
            writer: Mutex::new(writer),
            fields,
            card_rebuild_required: Mutex::new(card_rebuild_required),
            card_rebuild_marker,
        })
    }
    /// Return whether legacy card documents still need rebuilding from truth.
    pub fn needs_card_rebuild(&self) -> Result<bool, PortError> {
        self.card_rebuild_required
            .lock()
            .map(|required| *required)
            .map_err(|_| PortError::Internal {
                message: "card rebuild flag lock poisoned".to_string(),
            })
    }
    /// Mark a complete truth-backed card rebuild as durable.
    pub fn complete_card_rebuild(&self) -> Result<(), PortError> {
        if let Some(marker) = &self.card_rebuild_marker
            && marker.exists()
        {
            fs::remove_file(marker).map_err(to_io_port_error)?;
        }
        let mut required = self
            .card_rebuild_required
            .lock()
            .map_err(|_| PortError::Internal {
                message: "card rebuild flag lock poisoned".to_string(),
            })?;
        *required = false;
        Ok(())
    }
    fn chunk_document(&self, chunk: &IndexedChunk) -> TantivyDocument {
        doc!(
            self.fields.key => chunk_key(chunk.artifact_id, chunk.chunk_id),
            self.fields.artifact_id => chunk.artifact_id.value(),
            self.fields.chunk_id => chunk.chunk_id.value(),
            self.fields.text => chunk.text.clone(),
        )
    }

    fn lexical_chunk_document(&self, chunk: &IndexedLexicalChunk) -> TantivyDocument {
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

    fn read_chunk(&self, document: &TantivyDocument) -> Result<IndexedChunk, PortError> {
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
    fn read_lexical_chunk(
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

    fn card_document(&self, card: &IndexedCard) -> TantivyDocument {
        doc!(
            self.fields.card_key => card_key(card.artifact_id, card.card_id),
            self.fields.card_artifact_id => card.artifact_id.value(),
            self.fields.card_id => card.card_id.value(),
            self.fields.card_title => card.title.clone(),
            self.fields.card_body => card.body.clone(),
        )
    }

    fn lexical_card_document(&self, card: &IndexedLexicalCard) -> TantivyDocument {
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

    fn read_card(&self, document: &TantivyDocument) -> Result<IndexedCard, PortError> {
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
    fn read_lexical_card(
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

fn schema_field(schema: &Schema, name: &str) -> Result<Field, PortError> {
    schema.get_field(name).map_err(|_| PortError::Internal {
        message: format!("tantivy schema is missing {name} field"),
    })
}

fn chunk_key(artifact_id: ArtifactId, chunk_id: ChunkId) -> String {
    format!("{}:{}", artifact_id.value(), chunk_id.value())
}

fn card_key(artifact_id: ArtifactId, card_id: CardId) -> String {
    format!("card:{}:{}", artifact_id.value(), card_id.value())
}
fn descending_score(left: f32, right: f32) -> std::cmp::Ordering {
    match right.partial_cmp(&left) {
        Some(ordering) => ordering,
        None => std::cmp::Ordering::Equal,
    }
}

fn score_to_u32(score: f32) -> u32 {
    if !score.is_finite() || score <= 0.0 {
        return 0;
    }

    let scaled = score * 1_000.0;
    if scaled >= u32::MAX as f32 {
        u32::MAX
    } else {
        scaled.round() as u32
    }
}

fn to_port_error(error: tantivy::TantivyError) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
    }
}

fn to_io_port_error(error: std::io::Error) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod card_tests;
#[cfg(test)]
mod tests;
