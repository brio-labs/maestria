use crate::{
    error::{to_io_port_error, to_port_error},
    memory_document::BudgetedDocument,
    migration,
    migration::{legacy_chunks, schema_has_cards},
    ram_directory::{BudgetedRamDirectory, DirectoryCreditError},
    ram_memory::{InMemoryIndexError, MemoryBudgetError, RamMemoryBudget, RamMemoryReservation},
    schema::{IndexFields, load_fields, schema, supports_filtered_queries},
};
use sillage_governance::scan_secrets;
use sillage_ports::{FullTextIndex, PortError};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex, RwLock, RwLockReadGuard},
};
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, indexer::IndexWriterOptions,
    merge_policy::NoMergePolicy,
};

pub(super) const WRITER_MEMORY_BUDGET_BYTES: usize = 50_000_000;

/// Tantivy implementation of the [`FullTextIndex`] projection port.
pub struct TantivyFullTextIndex {
    pub(crate) index: Index,
    pub(crate) reader: IndexReader,
    pub(crate) reader_gate: RwLock<()>,
    pub(crate) writer: Mutex<Option<IndexWriter<BudgetedDocument>>>,
    pub(crate) fields: IndexFields,
    pub(crate) card_rebuild_required: Mutex<bool>,
    pub(crate) card_rebuild_marker: Option<std::path::PathBuf>,
    pub(crate) ram_directory: Option<BudgetedRamDirectory>,
    pub(crate) memory_budget: Option<Arc<RamMemoryBudget>>,
    _writer_reservation: Option<RamMemoryReservation>,
    pub(crate) writer_memory: Option<Mutex<WriterMemoryUsage>>,
    /// Set when the writer accepted operations that are not committed yet.
    dirty: AtomicBool,
}
pub(crate) struct WriterMemoryUsage {
    pub(crate) indexing_budget_bytes: u64,
    pub(crate) rehash_bytes: u64,
    pub(crate) max_document_bytes: u64,
    pub(crate) max_stored_document_bytes: u64,
    pub(crate) segment_documents: u64,
    pub(crate) historical_documents: u64,
    pub(crate) segment_tokens: u64,
    pub(crate) segment_terms: u64,
    pub(crate) delete_operations: u64,
    pub(crate) pending_delete_files_credit: u64,
    pub(crate) delete_queue_bytes: u64,
    // Reader/cache and ManagedDirectory path-set estimates retained to index destruction.
    pub(crate) reader_native_bytes: u64,
    pub(crate) reservation: RamMemoryReservation,
}

impl WriterMemoryUsage {
    pub(crate) fn new(
        indexing_budget_bytes: u64,
        rehash_bytes: u64,
        reservation: RamMemoryReservation,
    ) -> Self {
        Self {
            indexing_budget_bytes,
            rehash_bytes,
            max_document_bytes: 0,
            max_stored_document_bytes: 0,
            historical_documents: 0,
            segment_documents: 0,
            segment_tokens: 0,
            segment_terms: 0,
            delete_operations: 0,
            pending_delete_files_credit: 0,
            delete_queue_bytes: 0,
            reader_native_bytes: 65_536,
            reservation,
        }
    }
}

struct BudgetedIndexState {
    directory: BudgetedRamDirectory,
    budget: Arc<RamMemoryBudget>,
    writer_budget_bytes: usize,
    writer_reservation: RamMemoryReservation,
    writer_memory: WriterMemoryUsage,
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
            projection.commit_and_reload()?;
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
            fs::remove_dir_all(&backup_path).map_err(to_io_port_error)?;
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
        Self::from_index_configured(
            index,
            card_rebuild_required,
            card_rebuild_marker,
            read_only,
            ReloadPolicy::OnCommitWithDelay,
            None,
        )
    }

    /// Consume the first unreported admission refusal from the RAM directory or operation.
    /// Call after a failed port operation to distinguish quota refusal from native errors.
    pub fn take_memory_error(&self) -> Option<MemoryBudgetError> {
        self.ram_directory
            .as_ref()
            .and_then(BudgetedRamDirectory::take_memory_error)
    }

    pub(crate) fn from_budgeted_ram_index(
        index: Index,
        directory: BudgetedRamDirectory,
        budget: Arc<RamMemoryBudget>,
        writer_budget_bytes: usize,
        writer_reservation: RamMemoryReservation,
        writer_memory: WriterMemoryUsage,
    ) -> Result<Self, InMemoryIndexError> {
        let error_directory = directory.clone();
        match Self::from_index_configured(
            index,
            false,
            None,
            false,
            ReloadPolicy::Manual,
            Some(BudgetedIndexState {
                directory,
                budget,
                writer_budget_bytes,
                writer_reservation,
                writer_memory,
            }),
        ) {
            Ok(index) => Ok(index),
            Err(error) => match error_directory.take_memory_error() {
                Some(error) => Err(InMemoryIndexError::MemoryBudget(error)),
                None => Err(InMemoryIndexError::Index(error)),
            },
        }
    }

    fn from_index_configured(
        index: Index,
        card_rebuild_required: bool,
        card_rebuild_marker: Option<std::path::PathBuf>,
        read_only: bool,
        reload_policy: ReloadPolicy,
        budgeted: Option<BudgetedIndexState>,
    ) -> Result<Self, PortError> {
        let fields = load_fields(index.schema())?;
        let mut reader_builder = index.reader_builder().reload_policy(reload_policy);
        if budgeted.is_some() {
            reader_builder = reader_builder.doc_store_cache_num_blocks(0);
        }
        let reader = reader_builder.try_into().map_err(to_port_error)?;
        let writer = if read_only {
            None
        } else if let Some(state) = &budgeted {
            let options = IndexWriterOptions::builder()
                .num_worker_threads(1)
                .num_merge_threads(1)
                .memory_budget_per_thread(state.writer_budget_bytes)
                .build();
            let writer = index
                .writer_with_options::<BudgetedDocument>(options)
                .map_err(to_port_error)?;
            writer.set_merge_policy(Box::new(NoMergePolicy));
            Some(writer)
        } else {
            Some(
                index
                    .writer_with_num_threads::<BudgetedDocument>(1, WRITER_MEMORY_BUDGET_BYTES)
                    .map_err(to_port_error)?,
            )
        };
        let (ram_directory, memory_budget, writer_reservation, writer_memory) =
            if let Some(state) = budgeted {
                (
                    Some(state.directory),
                    Some(state.budget),
                    Some(state.writer_reservation),
                    Some(Mutex::new(state.writer_memory)),
                )
            } else {
                (None, None, None, None)
            };

        Ok(Self {
            index,
            reader,
            reader_gate: RwLock::new(()),
            writer: Mutex::new(writer),
            fields,
            card_rebuild_required: Mutex::new(card_rebuild_required),
            card_rebuild_marker,
            ram_directory,
            memory_budget,
            _writer_reservation: writer_reservation,
            writer_memory,
            dirty: AtomicBool::new(false),
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
        let writer_guard = self.writer.lock().map_err(|_| PortError::InternalContext {
            context: "Tantivy writer lock poisoned",
            source: "Tantivy writer mutex is poisoned".to_string(),
        })?;
        if writer_guard.is_none() {
            return Err(PortError::downstream(
                "complete card rebuild requires a writable full-text index",
                "full-text index is read-only",
            ));
        }
        let mut required =
            self.card_rebuild_required
                .lock()
                .map_err(|_| PortError::InternalContext {
                    context: "Tantivy card rebuild flag lock poisoned",
                    source: "card rebuild flag mutex is poisoned".to_string(),
                })?;
        if let Some(marker) = &self.card_rebuild_marker
            && marker.exists()
        {
            fs::remove_file(marker).map_err(to_io_port_error)?;
        }
        *required = false;
        Ok(())
    }

    pub(crate) fn with_writer<F>(&self, context: &'static str, f: F) -> Result<(), PortError>
    where
        F: FnOnce(&mut IndexWriter<BudgetedDocument>) -> Result<(), PortError>,
    {
        let mut writer_guard = self.writer.lock().map_err(|_| PortError::InternalContext {
            context: "Tantivy writer lock poisoned",
            source: "Tantivy writer mutex is poisoned".to_string(),
        })?;

        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| PortError::downstream(context, "full-text index is read-only"))?;
        f(writer)?;
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub(crate) fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub(crate) fn reader_guard(&self) -> Result<RwLockReadGuard<'_, ()>, PortError> {
        self.reader_gate
            .read()
            .map_err(|_| PortError::InternalContext {
                context: "Tantivy reader generation lock poisoned",
                source: "Tantivy reader generation lock is poisoned".to_string(),
            })
    }

    /// Number of documents currently committed in the index (drift signal
    /// for startup reconciliation watermarking; never flushes).
    pub fn doc_count(&self) -> Result<u64, PortError> {
        let _reader_guard = self.reader_guard()?;
        Ok(self.reader.searcher().num_docs())
    }

    /// Commit buffered writer operations and reload the reader when any are
    /// pending. Ingestion owns publication; search operations never commit.
    pub fn commit_and_reload(&self) -> Result<(), PortError> {
        if !self.dirty.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut writer_guard = self.writer.lock().map_err(|_| PortError::InternalContext {
            context: "Tantivy writer lock poisoned",
            source: "Tantivy writer mutex is poisoned".to_string(),
        })?;
        if !self.dirty.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(writer) = writer_guard.as_mut() else {
            return Err(PortError::downstream(
                "publish full-text index",
                "full-text index is read-only",
            ));
        };
        let _reader_guard = self
            .reader_gate
            .write()
            .map_err(|_| PortError::InternalContext {
                context: "Tantivy reader generation lock poisoned",
                source: "Tantivy reader generation lock is poisoned".to_string(),
            })?;
        let mut memory_guard = self
            .writer_memory
            .as_ref()
            .map(|memory| {
                memory.lock().map_err(|_| {
                    PortError::internal("Tantivy writer memory", "writer memory lock poisoned")
                })
            })
            .transpose()?;
        if let Some(memory) = memory_guard.as_mut() {
            self.reserve_commit_growth(memory)?;
        }
        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)?;
        if let Some(directory) = &self.ram_directory {
            directory
                .settle_output_credits()
                .map_err(|error| match error {
                    DirectoryCreditError::Memory(error) => self.memory_budget_error(error),
                    DirectoryCreditError::Poisoned => PortError::internal(
                        "budgeted RAM directory",
                        "directory accounting lock poisoned",
                    ),
                    DirectoryCreditError::ActiveScratch => PortError::internal(
                        "budgeted RAM directory",
                        "active write scratch remains after commit and reload",
                    ),
                })?;
        }
        if let Some(writer_memory) = memory_guard.as_mut() {
            writer_memory.segment_documents = 0;
            writer_memory.segment_tokens = 0;
            writer_memory.delete_operations = 0;
            writer_memory.pending_delete_files_credit = 0;
            writer_memory.segment_terms = 0;
            // Segment serialization has finished. Delete cursors can still retain old
            // blocks, and reader/native capacities are opaque: keep those charges until
            // the writer/index dies rather than equating commit with consumer destruction.
            let retained = writer_memory
                .rehash_bytes
                .checked_add(
                    writer_memory
                        .max_document_bytes
                        .saturating_sub(writer_memory.indexing_budget_bytes),
                )
                .and_then(|bytes| bytes.checked_add(writer_memory.delete_queue_bytes))
                .and_then(|bytes| bytes.checked_add(writer_memory.reader_native_bytes))
                .ok_or_else(|| self.accounting_overflow())?;
            writer_memory
                .reservation
                .resize(retained)
                .map_err(|error| self.memory_budget_error(error))?;
        }
        self.dirty.store(false, Ordering::Relaxed);
        Ok(())
    }
}
impl Drop for TantivyFullTextIndex {
    fn drop(&mut self) {
        if self.memory_budget.is_some() {
            // Ephemeral pending work has no publication consumer at destruction.
            return;
        }
        if let Err(error) = self.commit_and_reload() {
            tracing::warn!(%error, "tantivy index dropped with uncommittable buffered writes");
        }
    }
}
