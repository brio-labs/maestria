use crate::{InMemoryIndexError, RamMemoryBudget, TantivyFullTextIndex};

pub(crate) fn test_ram_index() -> Result<TantivyFullTextIndex, InMemoryIndexError> {
    let budget = std::sync::Arc::new(RamMemoryBudget::new(64 * 1024 * 1024));
    TantivyFullTextIndex::in_memory_with_budget(budget, 15_000_000)
}
