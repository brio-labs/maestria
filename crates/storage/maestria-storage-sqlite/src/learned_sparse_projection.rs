use std::sync::Arc;

use maestria_domain::IndexLifecycle;
use maestria_ports::{
    BoundedSearch, LearnedSparseIndex, LearnedSparseProjectionLifecycle, PortError, SparseDocument,
    SparseIdentity, SparseSearchHit, SparseSearchQuery,
};

use crate::SqliteStore;

mod lifecycle;
mod search;
mod search_storage;
mod storage;

/// Restartable SQLite-backed sparse projection bound to one complete identity.
///
/// The shared generation registry owns lifecycle decisions. This adapter mirrors
/// and validates those decisions durably, while rows remain rebuildable from
/// source truth and never become searchable before shadow/active state.
pub struct SqliteLearnedSparseIndex {
    store: Arc<SqliteStore>,
    identity: SparseIdentity,
}

impl SqliteLearnedSparseIndex {
    pub fn new(store: Arc<SqliteStore>, identity: SparseIdentity) -> Result<Self, PortError> {
        identity.validate()?;
        let index = Self { store, identity };
        storage::ensure_generation(&index.store, &index.identity)?;
        Ok(index)
    }
}

impl LearnedSparseIndex for SqliteLearnedSparseIndex {
    fn identity(&self) -> Option<SparseIdentity> {
        Some(self.identity.clone())
    }

    fn index_documents(&self, documents: Vec<SparseDocument>) -> Result<(), PortError> {
        storage::replace_documents(&self.store, &self.identity, &documents, false)
    }

    fn search(
        &self,
        query: SparseSearchQuery,
    ) -> Result<BoundedSearch<SparseSearchHit>, PortError> {
        self.search_filtered(query, &|_| Ok(true))
    }

    fn search_filtered(
        &self,
        query: SparseSearchQuery,
        filter: &dyn Fn(maestria_domain::ChunkId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<SparseSearchHit>, PortError> {
        search::execute(&self.store, &self.identity, query, filter)
    }

    fn delete_chunks(&self, chunk_ids: &[maestria_domain::ChunkId]) -> Result<(), PortError> {
        storage::tombstone_documents(&self.store, &self.identity, chunk_ids)
    }

    fn clear(&self) -> Result<(), PortError> {
        storage::clear_documents(&self.store, &self.identity)
    }

    fn rebuild(&self, documents: Vec<SparseDocument>) -> Result<(), PortError> {
        storage::replace_documents(&self.store, &self.identity, &documents, true)
    }
}

impl LearnedSparseProjectionLifecycle for SqliteLearnedSparseIndex {
    fn lifecycle(&self) -> Result<IndexLifecycle, PortError> {
        lifecycle::read(&self.store, &self.identity)
    }

    fn transition(&self, expected: IndexLifecycle, next: IndexLifecycle) -> Result<(), PortError> {
        lifecycle::transition(&self.store, &self.identity, expected, next)
    }

    fn collect(&self) -> Result<(), PortError> {
        lifecycle::collect(&self.store, &self.identity)
    }
}
