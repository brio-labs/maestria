use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

use maestria_domain::ChunkId;

/// Request-scoped bounded storage for records authorized during index filtering.
///
/// The cache is deliberately not shared across requests. The caller bounds
/// retained content-bearing records by the larger of the query limit and the
/// candidate budget, so every visited document's records stay resident and
/// final hits never re-run the authorization path.
pub(super) struct PrescoreCache<T> {
    capacity: usize,
    state: RefCell<PrescoreCacheState<T>>,
}

struct PrescoreCacheState<T> {
    entries: BTreeMap<ChunkId, T>,
    oldest: VecDeque<ChunkId>,
}

impl<T> PrescoreCache<T> {
    /// Creates a cache bounded by the caller's record budget.
    pub(super) fn new(record_budget: usize) -> Self {
        Self {
            capacity: record_budget.max(1),
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
        let PrescoreCacheState { entries, oldest } = &mut *state;
        if let Some(existing) = entries.get_mut(&chunk_id) {
            *existing = record;
            return;
        }
        while entries.len() >= self.capacity {
            if let Some(oldest_key) = oldest.pop_front() {
                if entries.remove(&oldest_key).is_some() {
                    break;
                }
            } else {
                break;
            }
        }
        if oldest.len() > self.capacity.saturating_mul(2) {
            oldest.retain(|id| entries.contains_key(id));
        }
        oldest.push_back(chunk_id);
        entries.insert(chunk_id, record);
    }

    /// Removes and returns a cached record. A miss is safe and returns `None`.
    pub(super) fn take(&self, chunk_id: ChunkId) -> Option<T> {
        let mut state = self.state.borrow_mut();
        state.entries.remove(&chunk_id)
    }
}
