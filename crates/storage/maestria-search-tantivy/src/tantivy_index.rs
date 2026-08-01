use crate::{
    error::{to_io_port_error, to_port_error},
    migration,
    migration::{legacy_chunks, schema_has_cards},
    schema::{IndexFields, load_fields, schema, supports_filtered_queries},
};
use maestria_governance::scan_secrets;
use maestria_ports::{FullTextIndex, PortError};
use std::{fs, path::Path, sync::Mutex};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy};

pub(super) const WRITER_MEMORY_BUDGET_BYTES: usize = 50_000_000;

/// Tantivy implementation of the [`FullTextIndex`] projection port.
pub struct TantivyFullTextIndex {
    pub(crate) index: Index,
    pub(crate) reader: IndexReader,
    pub(crate) writer: Mutex<Option<IndexWriter>>,
    pub(crate) fields: IndexFields,
    pub(crate) card_rebuild_required: Mutex<bool>,
    pub(crate) card_rebuild_marker: Option<std::path::PathBuf>,
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
                return Err(
                    match fs::rename(&backup_path, path).map_err(to_io_port_error) {
                        Err(rollback_error) => PortError::InternalContext {
                            context: "card rebuild migration failed and rollback of the original index also failed",
                            source: format!(
                                "migration rename: {error}; rollback rename: {rollback_error}"
                            ),
                        },
                        Ok(()) => to_io_port_error(error),
                    },
                );
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

    pub(crate) fn from_index(
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
            .map_err(|_| PortError::InternalContext {
                context: "Tantivy card rebuild flag lock poisoned",
                source: "card rebuild flag mutex is poisoned".to_string(),
            })
    }
    /// Mark a complete truth-backed card rebuild as durable.
    pub fn complete_card_rebuild(&self) -> Result<(), PortError> {
        if let Some(marker) = &self.card_rebuild_marker
            && marker.exists()
        {
            fs::remove_file(marker).map_err(to_io_port_error)?;
        }
        let mut required =
            self.card_rebuild_required
                .lock()
                .map_err(|_| PortError::InternalContext {
                    context: "Tantivy card rebuild flag lock poisoned",
                    source: "card rebuild flag mutex is poisoned".to_string(),
                })?;
        *required = false;
        Ok(())
    }
}
