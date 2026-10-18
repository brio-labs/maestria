use std::{mem::size_of, path::PathBuf};

use tantivy::directory::FileSlice;

use super::writer::{BudgetedRamWrite, StagingChunk};
use super::{BudgetedRamDirectory, DirectoryCreditError, DirectoryUsage, STAGING_CHUNK_BYTES};
use crate::ram_memory::{MemoryBudgetError, RamMemoryReservation};

struct ManagedCreditPlan {
    pending_base_paths: u64,
    retained_target: u64,
    scratch_target: u64,
    retained_delta: u64,
    scratch_delta: u64,
}

impl BudgetedRamDirectory {
    pub(crate) fn reserve_admission(
        &self,
        writer: &mut RamMemoryReservation,
        writer_required: u64,
        retained_delta: u64,
        scratch_delta: u64,
        managed_new_base_paths: u64,
        managed_delete_path_target: u64,
    ) -> Result<(), DirectoryCreditError> {
        let mut usage = self
            .usage
            .lock()
            .map_err(|_| DirectoryCreditError::Poisoned)?;
        let managed =
            self.managed_credit_plan(&usage, managed_new_base_paths, managed_delete_path_target)?;
        let retained_delta = retained_delta.checked_add(managed.retained_delta).ok_or(
            DirectoryCreditError::Memory(MemoryBudgetError::AccountingOverflow),
        )?;
        let scratch_delta = scratch_delta.checked_add(managed.scratch_delta).ok_or(
            DirectoryCreditError::Memory(MemoryBudgetError::AccountingOverflow),
        )?;
        let writer_delta = writer_required.saturating_sub(writer.reserved_bytes());
        let retained_credit_bytes = usage
            .retained_credit_bytes
            .checked_add(retained_delta)
            .ok_or(DirectoryCreditError::Memory(
                MemoryBudgetError::AccountingOverflow,
            ))?;
        let scratch_credit_bytes = usage
            .scratch_credit_bytes
            .checked_add(scratch_delta)
            .ok_or(DirectoryCreditError::Memory(
                MemoryBudgetError::AccountingOverflow,
            ))?;
        let output_delta =
            retained_delta
                .checked_add(scratch_delta)
                .ok_or(DirectoryCreditError::Memory(
                    MemoryBudgetError::AccountingOverflow,
                ))?;
        self.transfer_admission(writer, &mut usage, writer_delta, output_delta)?;
        usage.retained_credit_bytes = retained_credit_bytes;
        usage.scratch_credit_bytes = scratch_credit_bytes;
        usage.pending_managed_base_paths = managed.pending_base_paths;
        usage.managed_retained_credit_bytes = managed.retained_target;
        usage.managed_scratch_credit_bytes = managed.scratch_target;
        Ok(())
    }

    fn managed_credit_plan(
        &self,
        usage: &DirectoryUsage,
        new_base_paths: u64,
        delete_path_target: u64,
    ) -> Result<ManagedCreditPlan, DirectoryCreditError> {
        const MAX_MANAGED_PATH_BYTES: u64 = 57;
        let pending_base_paths = usage
            .pending_managed_base_paths
            .checked_add(new_base_paths)
            .ok_or(DirectoryCreditError::Memory(
                MemoryBudgetError::AccountingOverflow,
            ))?;
        let pending_delete_paths =
            delete_path_target.saturating_sub(usage.managed_delete_path_count);
        let pending_paths = pending_base_paths.checked_add(pending_delete_paths).ok_or(
            DirectoryCreditError::Memory(MemoryBudgetError::AccountingOverflow),
        )?;
        let (retained_target, scratch_target) = if pending_paths == 0 {
            (
                usage.managed_retained_credit_bytes,
                usage.managed_scratch_credit_bytes,
            )
        } else {
            let path_count = usage.managed_path_count.checked_add(pending_paths).ok_or(
                DirectoryCreditError::Memory(MemoryBudgetError::AccountingOverflow),
            )?;
            let data_bytes = path_count
                .checked_mul(MAX_MANAGED_PATH_BYTES + 3)
                .and_then(|bytes| bytes.checked_add(2))
                .ok_or(DirectoryCreditError::Memory(
                    MemoryBudgetError::AccountingOverflow,
                ))?;
            // Each new path rewrites the full list; its replacement overlaps the old slice.
            // Two maximum charges minus the current one bound all sequential rewrites.
            let charge = self
                .atomic_write_retained_bytes(data_bytes, 13)
                .map_err(DirectoryCreditError::Memory)?;
            let retained_target = charge
                .checked_mul(2)
                .ok_or(DirectoryCreditError::Memory(
                    MemoryBudgetError::AccountingOverflow,
                ))?
                .saturating_sub(usage.managed_json_charge_bytes)
                .max(usage.managed_retained_credit_bytes);
            (
                retained_target,
                data_bytes.max(usage.managed_scratch_credit_bytes),
            )
        };
        Ok(ManagedCreditPlan {
            pending_base_paths,
            retained_target,
            scratch_target,
            retained_delta: retained_target - usage.managed_retained_credit_bytes,
            scratch_delta: scratch_target - usage.managed_scratch_credit_bytes,
        })
    }

    fn transfer_admission(
        &self,
        writer: &mut RamMemoryReservation,
        usage: &mut DirectoryUsage,
        writer_delta: u64,
        output_delta: u64,
    ) -> Result<(), DirectoryCreditError> {
        if writer.reserved_bytes().checked_add(writer_delta).is_none()
            || usage
                .output_credit
                .reserved_bytes()
                .checked_add(output_delta)
                .is_none()
            || !writer.can_merge(&usage.output_credit)
        {
            return Err(DirectoryCreditError::Memory(
                MemoryBudgetError::AccountingOverflow,
            ));
        }
        let total_delta =
            writer_delta
                .checked_add(output_delta)
                .ok_or(DirectoryCreditError::Memory(
                    MemoryBudgetError::AccountingOverflow,
                ))?;
        let mut combined = self.budget.reserve(total_delta).map_err(|error| {
            self.remember_memory_error(error);
            DirectoryCreditError::Memory(error)
        })?;
        let Some(writer_part) = combined.split_off(writer_delta) else {
            return Err(DirectoryCreditError::Memory(
                MemoryBudgetError::AccountingOverflow,
            ));
        };
        let Some(output_part) = combined.split_off(output_delta) else {
            return Err(DirectoryCreditError::Memory(
                MemoryBudgetError::AccountingOverflow,
            ));
        };
        drop(combined);
        if let Err(writer_part) = writer.merge(writer_part) {
            drop(writer_part);
            return Err(DirectoryCreditError::Memory(
                MemoryBudgetError::AccountingOverflow,
            ));
        }
        if let Err(output_part) = usage.output_credit.merge(output_part) {
            if let Some(rollback) = writer.split_off(writer_delta) {
                drop(rollback);
            }
            drop(output_part);
            return Err(DirectoryCreditError::Memory(
                MemoryBudgetError::AccountingOverflow,
            ));
        }
        Ok(())
    }

    pub(crate) fn settle_output_credits(&self) -> Result<(), DirectoryCreditError> {
        let mut usage = self
            .usage
            .lock()
            .map_err(|_| DirectoryCreditError::Poisoned)?;
        if usage.active_scratch_bytes != 0 {
            return Err(DirectoryCreditError::ActiveScratch);
        }
        usage
            .output_credit
            .resize(0)
            .map_err(DirectoryCreditError::Memory)?;
        usage.retained_credit_bytes = 0;
        usage.scratch_credit_bytes = 0;
        usage.pending_managed_base_paths = 0;
        usage.managed_retained_credit_bytes = 0;
        usage.managed_scratch_credit_bytes = 0;
        Ok(())
    }

    pub(crate) fn estimate_file_output_credit(
        &self,
        payload_bytes: u64,
        file_count: u64,
    ) -> Result<(u64, u64), MemoryBudgetError> {
        const MAX_SEGMENT_PATH_BYTES: u64 = 57;
        let metadata_bytes = self
            .file_metadata_bytes(MAX_SEGMENT_PATH_BYTES)?
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_mul(file_count))
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let retained = payload_bytes
            .checked_add(metadata_bytes)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let writer_state = u64::try_from(size_of::<BudgetedRamWrite>())
            .map_err(|_| MemoryBudgetError::AccountingOverflow)?
            .checked_add(MAX_SEGMENT_PATH_BYTES)
            .and_then(|bytes| bytes.checked_mul(file_count))
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let chunk_bytes = u64::try_from(
            STAGING_CHUNK_BYTES
                .checked_add(size_of::<StagingChunk>())
                .and_then(|bytes| bytes.checked_add(2 * size_of::<usize>()))
                .ok_or(MemoryBudgetError::AccountingOverflow)?,
        )
        .map_err(|_| MemoryBudgetError::AccountingOverflow)?;
        let chunks = payload_bytes
            .div_ceil(STAGING_CHUNK_BYTES as u64)
            .checked_add(file_count)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let staging = chunk_bytes
            .checked_mul(chunks)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let scratch = writer_state
            .checked_add(staging)
            .and_then(|bytes| bytes.checked_add(payload_bytes))
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        Ok((retained, scratch))
    }

    pub(crate) fn atomic_write_retained_bytes(
        &self,
        payload_bytes: u64,
        path_bytes: u64,
    ) -> Result<u64, MemoryBudgetError> {
        payload_bytes
            .checked_add(self.file_metadata_bytes(path_bytes)?)
            .ok_or(MemoryBudgetError::AccountingOverflow)
    }

    pub(crate) fn file_handle_bytes(&self) -> Result<u64, MemoryBudgetError> {
        let bytes = size_of::<FileSlice>()
            .checked_add(2 * size_of::<usize>())
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        u64::try_from(bytes).map_err(|_| MemoryBudgetError::AccountingOverflow)
    }

    fn file_metadata_bytes(&self, path_bytes: u64) -> Result<u64, MemoryBudgetError> {
        let file_slices = 2usize
            .checked_mul(size_of::<(PathBuf, FileSlice)>())
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let wrappers = 4usize
            .checked_mul(size_of::<usize>())
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let metadata = u64::try_from(file_slices)
            .ok()
            .and_then(|bytes| bytes.checked_add(u64::try_from(wrappers).ok()?))
            .and_then(|bytes| bytes.checked_add(path_bytes))
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        Ok(metadata)
    }
}
