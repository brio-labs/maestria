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
    cache: std::sync::Mutex<Option<search::SearchCache>>,
}

impl SqliteLearnedSparseIndex {
    pub fn new(store: Arc<SqliteStore>, identity: SparseIdentity) -> Result<Self, PortError> {
        identity.validate()?;
        let index = Self {
            store,
            identity,
            cache: std::sync::Mutex::new(None),
        };
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
        let identity_json = storage::identity_json(&self.identity)?;
        let version = storage::read_version(&self.store, &identity_json)?;
        let Some(version) = version else {
            // A projection written before the version row: no cache, cold
            // per-document reads.
            return search::execute(&self.store, &self.identity, query, filter);
        };
        let cached = {
            let mut cache = self.cache.lock().map_err(|_| PortError::InternalContext {
                context: "sparse search cache",
                source: "cache lock is poisoned".to_string(),
            })?;
            match cache.as_ref() {
                Some(cached) if cached.version == version => cached.clone(),
                _ => {
                    let documents =
                        Arc::new(storage::load_all_documents(&self.store, &self.identity)?);
                    let mut postings = std::collections::BTreeMap::<u32, Vec<usize>>::new();
                    for (index, cached) in documents.iter().enumerate() {
                        for term in cached.document.vector.terms() {
                            postings.entry(term.term_id()).or_default().push(index);
                        }
                    }
                    let cached = search::SearchCache {
                        version,
                        documents,
                        postings: Arc::new(postings),
                    };
                    *cache = Some(cached.clone());
                    cached
                }
            }
        };
        search::execute_cached(
            cached.documents.as_ref(),
            cached.postings.as_ref(),
            &self.identity,
            &self.store,
            query,
            filter,
        )
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
