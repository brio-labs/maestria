#![forbid(unsafe_code)]

//! Tantivy-backed full-text search projection for Maestria.
//!
//! This crate stores only rebuildable indexed chunks. Artifact metadata and blob
//! contents remain owned by their source repositories.

/// Responsibility map:
/// - `constructors`: module responsibility.
/// - `lexical_helpers`: module responsibility.
/// - `lexical_operations`: module responsibility.
/// - `migration`: module responsibility.
/// - `operations`: module responsibility.
/// - `schema`: module responsibility.
/// - `search_helpers`: module responsibility.
mod constructors;
mod documents;
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
use maestria_ports::{FullTextIndex, PortError};
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy,
    schema::{Field, Schema},
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
    writer: Mutex<Option<IndexWriter>>,
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
                return Self::from_index(existing, required, Some(marker), false);
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
            let projection = Self::from_index(rebuilt, true, Some(temp_marker), false)?;
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
            let projection = Self::from_index(migrated, true, Some(marker), false);
            let _ = fs::remove_dir_all(&backup_path);
            return projection;
        }

        let index = Index::create_in_dir(path, schema()).map_err(to_port_error)?;
        let required = marker.exists();
        Self::from_index(index, required, Some(marker), false)
    }

    fn from_index(
        index: Index,
        card_rebuild_required: bool,
        card_rebuild_marker: Option<std::path::PathBuf>,
        read_only: bool,
    ) -> Result<Self, PortError> {
        let fields = load_fields(index.schema())?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(to_port_error)?;
        let writer = if read_only {
            None
        } else {
            Some(
                index
                    .writer_with_num_threads(1, WRITER_MEMORY_BUDGET_BYTES)
                    .map_err(to_port_error)?,
            )
        };

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
