use std::{
    fmt, io,
    mem::size_of,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tantivy::directory::{Directory, FileSlice};

use crate::ram_memory::{MemoryBudgetError, RamMemoryBudget, RamMemoryReservation};

#[path = "ram_directory_admission.rs"]
mod admission;
#[path = "ram_directory_io.rs"]
mod io_adapter;
#[path = "ram_directory_scratch.rs"]
mod scratch;
#[path = "ram_directory_writer.rs"]
mod writer;

const STAGING_CHUNK_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub(crate) struct BudgetedRamDirectory {
    inner: tantivy::directory::RamDirectory,
    budget: Arc<RamMemoryBudget>,
    usage: Arc<Mutex<DirectoryUsage>>,
    memory_error: Arc<DirectoryMemoryError>,
}

struct DirectoryUsage {
    // ManagedDirectory loads `.managed.json` on wrap only; no later handle escapes.
    // Other native generations stay charged until their last consumer is known gone.
    retained: RamMemoryReservation,
    output_credit: RamMemoryReservation,
    retained_credit_bytes: u64,
    scratch_credit_bytes: u64,
    active_scratch_bytes: u64,
    managed_path_count: u64,
    managed_delete_path_count: u64,
    pending_managed_base_paths: u64,
    managed_json_charge_bytes: u64,
    managed_retained_credit_bytes: u64,
    managed_scratch_credit_bytes: u64,
}
struct DirectoryMemoryError {
    error: Mutex<Option<MemoryBudgetError>>,
    _reservation: RamMemoryReservation,
}

pub(crate) enum DirectoryCreditError {
    Memory(MemoryBudgetError),
    Poisoned,
    ActiveScratch,
}

impl BudgetedRamDirectory {
    pub(crate) fn new(budget: Arc<RamMemoryBudget>) -> Result<Self, MemoryBudgetError> {
        // Native map header estimate: RandomState plus hashbrown's control pointer
        // and three word-sized counters. Empty tables allocate no buckets. The
        // lock/padding and existing 34-word watcher/Arc allowance remain estimates,
        // not a substitute for measured native allocations or RSS.
        let map_header_bytes = size_of::<std::hash::RandomState>()
            .checked_add(4 * size_of::<usize>())
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let inner_bytes = size_of::<std::sync::RwLock<()>>()
            .checked_add(map_header_bytes)
            .and_then(|bytes| bytes.checked_add(size_of::<usize>()))
            .and_then(|bytes| bytes.checked_add(34 * size_of::<usize>()))
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let usage_bytes = size_of::<Mutex<DirectoryUsage>>()
            .checked_add(2 * size_of::<usize>())
            .and_then(|bytes| bytes.checked_add(inner_bytes))
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let usage_bytes =
            u64::try_from(usage_bytes).map_err(|_| MemoryBudgetError::AccountingOverflow)?;
        let error_bytes = size_of::<DirectoryMemoryError>()
            .checked_add(2 * size_of::<usize>())
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let error_bytes =
            u64::try_from(error_bytes).map_err(|_| MemoryBudgetError::AccountingOverflow)?;
        let usage_reservation = budget.reserve(usage_bytes)?;
        let error_reservation = budget.reserve(error_bytes)?;
        let output_credit = budget.reserve(0)?;
        Ok(Self {
            inner: tantivy::directory::RamDirectory::create(),
            budget,
            usage: Arc::new(Mutex::new(DirectoryUsage {
                retained: usage_reservation,
                output_credit,
                retained_credit_bytes: 0,
                scratch_credit_bytes: 0,
                active_scratch_bytes: 0,
                managed_path_count: 0,
                managed_delete_path_count: 0,
                pending_managed_base_paths: 0,
                managed_json_charge_bytes: 0,
                managed_retained_credit_bytes: 0,
                managed_scratch_credit_bytes: 0,
            })),
            memory_error: Arc::new(DirectoryMemoryError {
                error: Mutex::new(None),
                _reservation: error_reservation,
            }),
        })
    }

    pub(crate) fn take_memory_error(&self) -> Option<MemoryBudgetError> {
        self.memory_error
            .error
            .lock()
            .ok()
            .and_then(|mut error| error.take())
    }

    pub(crate) fn remember_memory_error(&self, error: MemoryBudgetError) {
        if let Ok(mut slot) = self.memory_error.error.lock()
            && slot.is_none()
        {
            *slot = Some(error);
        }
    }

    fn lock_usage(&self) -> io::Result<std::sync::MutexGuard<'_, DirectoryUsage>> {
        self.usage.lock().map_err(|error| {
            io::Error::other(format!("RAM directory accounting lock poisoned: {error}"))
        })
    }

    fn retained_charge_locked(
        &self,
        usage: &mut DirectoryUsage,
        additional: u64,
        managed_metadata: bool,
    ) -> io::Result<()> {
        if usage
            .retained
            .reserved_bytes()
            .checked_add(additional)
            .is_none()
            || usage.managed_retained_credit_bytes > usage.retained_credit_bytes
        {
            return Err(self.io_memory_error(MemoryBudgetError::AccountingOverflow));
        }
        let managed_credited = if managed_metadata {
            additional.min(usage.managed_retained_credit_bytes)
        } else {
            0
        };
        let general_credit = usage.retained_credit_bytes - usage.managed_retained_credit_bytes;
        let general_credited = (additional - managed_credited).min(general_credit);
        let credited = managed_credited
            .checked_add(general_credited)
            .ok_or_else(|| self.io_memory_error(MemoryBudgetError::AccountingOverflow))?;
        let remaining_credit = usage
            .retained_credit_bytes
            .checked_sub(credited)
            .ok_or_else(|| self.io_memory_error(MemoryBudgetError::AccountingOverflow))?;
        let remaining_managed_credit = usage
            .managed_retained_credit_bytes
            .checked_sub(managed_credited)
            .ok_or_else(|| self.io_memory_error(MemoryBudgetError::AccountingOverflow))?;
        let additional_global = additional - credited;
        if usage.output_credit.reserved_bytes() < usage.retained_credit_bytes
            || !usage.retained.can_merge(&usage.output_credit)
        {
            return Err(self.io_memory_error(MemoryBudgetError::AccountingOverflow));
        }
        let direct = self.budget.reserve(additional_global).map_err(|error| {
            self.remember_memory_error(error);
            io::Error::new(io::ErrorKind::OutOfMemory, error)
        })?;
        let Some(credit) = usage.output_credit.split_off(credited) else {
            return Err(self.io_memory_error(MemoryBudgetError::AccountingOverflow));
        };
        if let Err(credit) = usage.retained.merge(credit) {
            if let Err(credit) = usage.output_credit.merge(credit) {
                drop(credit);
            }
            return Err(self.io_memory_error(MemoryBudgetError::AccountingOverflow));
        }
        if let Err(direct) = usage.retained.merge(direct) {
            if let Some(credit) = usage.retained.split_off(credited)
                && let Err(credit) = usage.output_credit.merge(credit)
            {
                drop(credit);
            }
            drop(direct);
            return Err(self.io_memory_error(MemoryBudgetError::AccountingOverflow));
        }
        usage.retained_credit_bytes = remaining_credit;
        usage.managed_retained_credit_bytes = remaining_managed_credit;
        Ok(())
    }

    fn write_file_locked(
        &self,
        usage: &mut DirectoryUsage,
        path: &Path,
        data: &[u8],
    ) -> io::Result<()> {
        let metadata_bytes = path
            .as_os_str()
            .len()
            .checked_add(size_of::<(PathBuf, FileSlice)>())
            .and_then(|bytes| bytes.checked_add(size_of::<(PathBuf, FileSlice)>()))
            .and_then(|bytes| bytes.checked_add(4 * size_of::<usize>()))
            .ok_or_else(|| self.io_memory_error(MemoryBudgetError::AccountingOverflow))?;
        let data_bytes = u64::try_from(data.len())
            .map_err(|_| self.io_memory_error(MemoryBudgetError::AccountingOverflow))?;
        let retained_bytes = u64::try_from(metadata_bytes)
            .ok()
            .and_then(|metadata| metadata.checked_add(data_bytes))
            .ok_or_else(|| self.io_memory_error(MemoryBudgetError::AccountingOverflow))?;
        let managed_metadata = path == Path::new(".managed.json");
        let old_managed_charge = if managed_metadata {
            usage.managed_json_charge_bytes
        } else {
            0
        };
        // File writes can interleave after ManagedDirectory updates this persisted list.
        let (managed_path_count, managed_delete_path_count, new_base_paths) = if managed_metadata {
            let (path_count, delete_count) = self.count_managed_paths(data)?;
            let added_paths = path_count.saturating_sub(usage.managed_path_count);
            let added_delete_paths = delete_count.saturating_sub(usage.managed_delete_path_count);
            (
                path_count,
                delete_count,
                added_paths.saturating_sub(added_delete_paths),
            )
        } else {
            (0, 0, 0)
        };
        self.retained_charge_locked(usage, retained_bytes, managed_metadata)?;
        let recycled_retained_credit = usage
            .retained_credit_bytes
            .checked_add(old_managed_charge)
            .ok_or_else(|| self.io_memory_error(MemoryBudgetError::AccountingOverflow))?;
        let recycled_managed_credit = usage
            .managed_retained_credit_bytes
            .checked_add(old_managed_charge)
            .ok_or_else(|| self.io_memory_error(MemoryBudgetError::AccountingOverflow))?;
        if managed_metadata
            && (usage.retained.reserved_bytes() < old_managed_charge
                || usage
                    .output_credit
                    .reserved_bytes()
                    .checked_add(old_managed_charge)
                    .is_none()
                || !usage.retained.can_merge(&usage.output_credit))
        {
            return Err(self.io_memory_error(MemoryBudgetError::AccountingOverflow));
        }
        // The managed list is only read when Tantivy wraps the directory; replacing its
        // map entry drops the old FileSlice after this write succeeds.
        self.inner.atomic_write(path, data)?;
        if managed_metadata {
            if old_managed_charge != 0 {
                let Some(old_charge) = usage.retained.split_off(old_managed_charge) else {
                    return Err(self.io_memory_error(MemoryBudgetError::AccountingOverflow));
                };
                if let Err(old_charge) = usage.output_credit.merge(old_charge) {
                    drop(old_charge);
                    return Err(self.io_memory_error(MemoryBudgetError::AccountingOverflow));
                }
            }
            usage.retained_credit_bytes = recycled_retained_credit;
            usage.managed_retained_credit_bytes = recycled_managed_credit;
            usage.managed_json_charge_bytes = retained_bytes;
            usage.pending_managed_base_paths = usage
                .pending_managed_base_paths
                .saturating_sub(new_base_paths);
            usage.managed_path_count = managed_path_count;
            usage.managed_delete_path_count = managed_delete_path_count;
        }
        Ok(())
    }

    fn count_managed_paths(&self, data: &[u8]) -> io::Result<(u64, u64)> {
        let mut path_count = 0u64;
        let mut delete_path_count = 0u64;
        let mut string_start = None;
        // Tantivy's managed file paths are ASCII segment names with no JSON escapes.
        for (index, byte) in data.iter().enumerate() {
            if *byte != b'"' {
                continue;
            }
            if let Some(start) = string_start.take() {
                path_count = path_count
                    .checked_add(1)
                    .ok_or_else(|| self.io_memory_error(MemoryBudgetError::AccountingOverflow))?;
                if data[start..index].ends_with(b".del") {
                    delete_path_count = delete_path_count.checked_add(1).ok_or_else(|| {
                        self.io_memory_error(MemoryBudgetError::AccountingOverflow)
                    })?;
                }
            } else {
                string_start = Some(index + 1);
            }
        }
        Ok((path_count, delete_path_count))
    }

    fn atomic_write_precharged(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        let mut usage = self.lock_usage()?;
        self.write_file_locked(&mut usage, path, data)
    }

    fn io_memory_error(&self, error: MemoryBudgetError) -> io::Error {
        self.remember_memory_error(error);
        io::Error::new(io::ErrorKind::OutOfMemory, error)
    }
}

impl fmt::Debug for BudgetedRamDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BudgetedRamDirectory")
    }
}
