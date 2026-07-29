use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

use maestria_domain::ChunkId;

/// Request-scoped bounded storage for records authorized during index filtering.
///
/// The cache is deliberately not shared across requests. A validated positive query
/// limit is supplied by each caller, so it bounds retained content-bearing records.
pub(super) struct PrescoreCache<T> {
    capacity: usize,
    state: RefCell<PrescoreCacheState<T>>,
}

struct PrescoreCacheState<T> {
    entries: BTreeMap<ChunkId, T>,
    oldest: VecDeque<ChunkId>,
}

impl<T> PrescoreCache<T> {
    /// Creates a cache bounded by the validated query limit.
    pub(super) fn new(validated_query_limit: usize) -> Self {
        Self {
            capacity: validated_query_limit.max(1),
            state: RefCell::new(PrescoreCacheState {
                entries: BTreeMap::new(),
                oldest: VecDeque::new(),
            }),
        }
    }

    /// Inserts a record, replacing an existing record for the same chunk and
    /// evicting the oldest distinct entry when the bound is reached.
    pub(super) fn insert(&self, chunk_id: ChunkId, record: T) {
        let mut state = self.state.borrow_mut();
        if let Some(existing) = state.entries.get_mut(&chunk_id) {
            *existing = record;
            return;
        }
        if state.entries.len() >= self.capacity
            && let Some(oldest) = state.oldest.pop_front()
        {
            state.entries.remove(&oldest);
        }
        state.oldest.push_back(chunk_id);
        state.entries.insert(chunk_id, record);
    }

    /// Removes and returns a cached record. A miss is safe and returns `None`.
    pub(super) fn take(&self, chunk_id: ChunkId) -> Option<T> {
        let mut state = self.state.borrow_mut();
        let record = state.entries.remove(&chunk_id)?;
        if let Some(position) = state.oldest.iter().position(|id| *id == chunk_id) {
            state.oldest.remove(position);
        }
        Some(record)
    }
}
