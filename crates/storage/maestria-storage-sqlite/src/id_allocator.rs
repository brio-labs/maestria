use maestria_domain::{ApprovalId, ClaimId, MemoryCandidateId};
use maestria_ports::{IdAllocator, PortError};
use rusqlite::params;

use crate::sqlite_store::map_append_error;

fn allocate_id<T>(
    store: &crate::SqliteStore,
    namespace: &'static str,
    context: &'static str,
    make: fn(u64) -> T,
) -> Result<T, PortError> {
    store.with_transaction(|transaction| {
        crate::schema::seed_id_counters(transaction)?;
        let next: i64 = transaction
            .query_row(
                "UPDATE id_counters SET next_id = next_id + 1 WHERE namespace = ?1 RETURNING next_id - 1",
                params![namespace],
                |row| row.get(0),
            )
            .map_err(map_append_error)?;
        let id = u64::try_from(next).map_err(|_| {
            PortError::internal(context, format!("{namespace} id counter overflow"))
        })?;
        Ok(make(id))
    })
}

impl IdAllocator for crate::SqliteStore {
    fn allocate_claim_id(&self) -> Result<ClaimId, PortError> {
        allocate_id(self, "claim", "allocate claim id", ClaimId::new)
    }

    fn allocate_memory_candidate_id(&self) -> Result<MemoryCandidateId, PortError> {
        allocate_id(
            self,
            "memory_candidate",
            "allocate memory candidate id",
            MemoryCandidateId::new,
        )
    }

    fn allocate_approval_id(&self) -> Result<ApprovalId, PortError> {
        allocate_id(self, "approval", "allocate approval id", ApprovalId::new)
    }
}
