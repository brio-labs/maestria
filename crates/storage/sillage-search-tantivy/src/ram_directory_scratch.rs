use super::{BudgetedRamDirectory, DirectoryUsage};
use crate::ram_memory::{MemoryBudgetError, RamMemoryReservation};

pub(super) struct ScratchReservation {
    directory: BudgetedRamDirectory,
    reservation: Option<RamMemoryReservation>,
}

impl ScratchReservation {
    pub(super) fn grow(&mut self, bytes: u64) -> std::io::Result<()> {
        if bytes == 0 {
            return Ok(());
        }
        let Some(reservation) = self.reservation.as_mut() else {
            return Err(std::io::Error::other(
                "RAM scratch reservation is unavailable",
            ));
        };
        self.directory.grow_scratch(reservation, bytes)
    }
}

impl Drop for ScratchReservation {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            self.directory.return_scratch(reservation);
        }
    }
}

impl BudgetedRamDirectory {
    pub(super) fn scratch_reservation(&self, bytes: u64) -> std::io::Result<ScratchReservation> {
        let mut usage = self.lock_usage()?;
        let reservation = self
            .take_scratch_locked(&mut usage, bytes)
            .map_err(|error| self.io_memory_error(error))?;
        Ok(ScratchReservation {
            directory: self.clone(),
            reservation: Some(reservation),
        })
    }

    fn take_scratch_locked(
        &self,
        usage: &mut DirectoryUsage,
        bytes: u64,
    ) -> Result<RamMemoryReservation, MemoryBudgetError> {
        let output_pool = usage
            .output_credit
            .reserved_bytes()
            .checked_sub(usage.retained_credit_bytes)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let scratch_pool = usage
            .scratch_credit_bytes
            .checked_sub(usage.active_scratch_bytes)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        if output_pool != scratch_pool {
            return Err(MemoryBudgetError::AccountingOverflow);
        }
        let from_pool = bytes.min(scratch_pool);
        let direct_bytes = bytes - from_pool;
        let active_scratch_bytes = usage
            .active_scratch_bytes
            .checked_add(bytes)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let scratch_credit_bytes = usage
            .scratch_credit_bytes
            .checked_add(direct_bytes)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let mut reservation = self.budget.reserve(direct_bytes)?;
        if !reservation.can_merge(&usage.output_credit) {
            return Err(MemoryBudgetError::AccountingOverflow);
        }
        let Some(pool_reservation) = usage.output_credit.split_off(from_pool) else {
            return Err(MemoryBudgetError::AccountingOverflow);
        };
        if let Err(pool_reservation) = reservation.merge(pool_reservation) {
            if let Err(pool_reservation) = usage.output_credit.merge(pool_reservation) {
                drop(pool_reservation);
            }
            return Err(MemoryBudgetError::AccountingOverflow);
        }
        usage.active_scratch_bytes = active_scratch_bytes;
        usage.scratch_credit_bytes = scratch_credit_bytes;
        Ok(reservation)
    }

    pub(super) fn grow_scratch(
        &self,
        reservation: &mut RamMemoryReservation,
        additional: u64,
    ) -> std::io::Result<()> {
        let mut usage = self.lock_usage()?;
        let extra = self
            .take_scratch_locked(&mut usage, additional)
            .map_err(|error| self.io_memory_error(error))?;
        if !reservation.can_merge(&extra) {
            self.return_scratch_locked(&mut usage, extra);
            return Err(self.io_memory_error(MemoryBudgetError::AccountingOverflow));
        }
        if let Err(extra) = reservation.merge(extra) {
            self.return_scratch_locked(&mut usage, extra);
            return Err(self.io_memory_error(MemoryBudgetError::AccountingOverflow));
        }
        Ok(())
    }

    fn return_scratch_locked(&self, usage: &mut DirectoryUsage, reservation: RamMemoryReservation) {
        let bytes = reservation.reserved_bytes();
        let Some(active_scratch_bytes) = usage.active_scratch_bytes.checked_sub(bytes) else {
            self.remember_memory_error(MemoryBudgetError::AccountingOverflow);
            drop(reservation);
            return;
        };
        if !usage.output_credit.can_merge(&reservation) {
            usage.active_scratch_bytes = active_scratch_bytes;
            usage.scratch_credit_bytes = usage.scratch_credit_bytes.saturating_sub(bytes);
            self.remember_memory_error(MemoryBudgetError::AccountingOverflow);
            drop(reservation);
            return;
        }
        if let Err(reservation) = usage.output_credit.merge(reservation) {
            usage.active_scratch_bytes = active_scratch_bytes;
            usage.scratch_credit_bytes = usage.scratch_credit_bytes.saturating_sub(bytes);
            self.remember_memory_error(MemoryBudgetError::AccountingOverflow);
            drop(reservation);
            return;
        }
        usage.active_scratch_bytes = active_scratch_bytes;
    }

    fn return_scratch(&self, reservation: RamMemoryReservation) {
        match self.usage.lock() {
            Ok(mut usage) => self.return_scratch_locked(&mut usage, reservation),
            Err(_) => drop(reservation),
        }
    }
}
