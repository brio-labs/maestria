use crate::{
    error::to_port_error, memory_document::BudgetedDocument, ram_directory::DirectoryCreditError,
    tantivy_index::TantivyFullTextIndex,
};
use sillage_ports::PortError;
use std::sync::{RwLockReadGuard, atomic::Ordering};
use tantivy::IndexWriter;

impl TantivyFullTextIndex {
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
