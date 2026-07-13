use maestria_domain::{ApprovalId, ClaimId, MemoryCandidateId};
use maestria_ports::{IdAllocator, PortError};

use crate::{map_append_error, to_port_error};

impl IdAllocator for crate::SqliteStore {
    fn allocate_claim_id(&self) -> Result<ClaimId, PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        crate::schema::seed_id_counters(&transaction)?;

        let next: i64 = transaction
            .query_row(
                "UPDATE id_counters SET next_id = next_id + 1 WHERE namespace = 'claim' RETURNING next_id - 1",
                [],
                |row| row.get(0),
            )
            .map_err(map_append_error)?;

        transaction.commit().map_err(to_port_error)?;

        let id = u64::try_from(next).map_err(|_| PortError::Internal {
            message: "claim id counter overflow".to_string(),
        })?;
        Ok(ClaimId::new(id))
    }

    fn allocate_memory_candidate_id(&self) -> Result<MemoryCandidateId, PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        crate::schema::seed_id_counters(&transaction)?;

        let next: i64 = transaction
            .query_row(
                "UPDATE id_counters SET next_id = next_id + 1 WHERE namespace = 'memory_candidate' RETURNING next_id - 1",
                [],
                |row| row.get(0),
            )
            .map_err(map_append_error)?;

        transaction.commit().map_err(to_port_error)?;

        let id = u64::try_from(next).map_err(|_| PortError::Internal {
            message: "memory candidate id counter overflow".to_string(),
        })?;
        Ok(MemoryCandidateId::new(id))
    }

    fn allocate_approval_id(&self) -> Result<ApprovalId, PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        crate::schema::seed_id_counters(&transaction)?;

        let next: i64 = transaction
            .query_row(
                "UPDATE id_counters SET next_id = next_id + 1 WHERE namespace = 'approval' RETURNING next_id - 1",
                [],
                |row| row.get(0),
            )
            .map_err(map_append_error)?;

        transaction.commit().map_err(to_port_error)?;

        let id = u64::try_from(next).map_err(|_| PortError::Internal {
            message: "approval id counter overflow".to_string(),
        })?;
        Ok(ApprovalId::new(id))
    }
}
