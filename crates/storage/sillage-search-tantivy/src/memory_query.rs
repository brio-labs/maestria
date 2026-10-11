//! Admission for query parser/scorer/collector temporaries and output construction.
//! Opaque native estimates are declared estimates, never RSS enforcement.

use crate::{ram_memory::RamMemoryReservation, tantivy_index::TantivyFullTextIndex};
use sillage_ports::{PortError, SearchQuery};

impl TantivyFullTextIndex {
    pub(crate) fn reserve_query_work(
        &self,
        query: &SearchQuery,
        documents: u64,
    ) -> Result<Option<RamMemoryReservation>, PortError> {
        if self.memory_budget.is_none() {
            return Ok(None);
        }
        // Simultaneous collector/scored vectors, B-tree authorization keys, term-set
        // copies, sorting scratch, fast-field readers and opaque scorer state.
        let candidates = documents;
        let bytes = candidates
            .checked_mul(512)
            .and_then(|bytes| bytes.checked_add((query.q.len() as u64).checked_mul(64)?))
            .and_then(|bytes| bytes.checked_add(65_536))
            .ok_or_else(|| self.accounting_overflow())?;
        self.reserve_document_memory(bytes)
    }

    pub(crate) fn reserve_scored_output(
        &self,
        documents: usize,
    ) -> Result<Option<RamMemoryReservation>, PortError> {
        if self.memory_budget.is_none() {
            return Ok(None);
        }
        let bound = self.search_output_bound()?;
        let bytes = u64::try_from(documents)
            .ok()
            .and_then(|count| count.checked_mul(bound))
            .ok_or_else(|| self.accounting_overflow())?;
        self.reserve_search_output(bytes)
    }
}
