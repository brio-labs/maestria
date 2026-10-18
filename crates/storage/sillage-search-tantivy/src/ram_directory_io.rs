use std::{
    io::{self, BufWriter},
    mem::size_of,
    path::Path,
    sync::Arc,
};

use tantivy::directory::{
    Directory, FileHandle, FileSlice, WatchCallback, WatchHandle, WritePtr,
    error::{DeleteError, OpenReadError, OpenWriteError},
};

use tantivy::HasLen;

use super::{BudgetedRamDirectory, writer::BudgetedRamWrite};
use crate::ram_memory::MemoryBudgetError;

impl Directory for BudgetedRamDirectory {
    fn get_file_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        let file = self.inner.open_read(path)?;
        let handle_bytes = size_of::<FileSlice>()
            .checked_add(2 * size_of::<usize>())
            .ok_or_else(|| {
                OpenReadError::wrap_io_error(
                    self.io_memory_error(MemoryBudgetError::AccountingOverflow),
                    path.to_path_buf(),
                )
            })?;
        let handle_bytes = u64::try_from(handle_bytes).map_err(|_| {
            OpenReadError::wrap_io_error(
                self.io_memory_error(MemoryBudgetError::AccountingOverflow),
                path.to_path_buf(),
            )
        })?;
        let mut usage = self
            .lock_usage()
            .map_err(|error| OpenReadError::wrap_io_error(error, path.to_path_buf()))?;
        self.retained_charge_locked(&mut usage, handle_bytes, false)
            .map_err(|error| OpenReadError::wrap_io_error(error, path.to_path_buf()))?;
        Ok(Arc::new(file))
    }

    fn delete(&self, path: &Path) -> Result<(), DeleteError> {
        self.inner.delete(path)
    }

    fn exists(&self, path: &Path) -> Result<bool, OpenReadError> {
        self.inner.exists(path)
    }

    fn open_write(&self, path: &Path) -> Result<WritePtr, OpenWriteError> {
        let path_bytes = path.as_os_str().len();
        let writer_bytes = size_of::<BudgetedRamWrite>()
            .checked_add(path_bytes)
            .ok_or_else(|| {
                OpenWriteError::wrap_io_error(
                    self.io_memory_error(MemoryBudgetError::AccountingOverflow),
                    path.to_path_buf(),
                )
            })?;
        let writer_bytes = u64::try_from(writer_bytes).map_err(|_| {
            OpenWriteError::wrap_io_error(
                self.io_memory_error(MemoryBudgetError::AccountingOverflow),
                path.to_path_buf(),
            )
        })?;
        let writer_reservation = self
            .scratch_reservation(writer_bytes)
            .map_err(|error| OpenWriteError::wrap_io_error(error, path.to_path_buf()))?;
        let path_buf = path.to_path_buf();
        let mut usage = self
            .lock_usage()
            .map_err(|error| OpenWriteError::wrap_io_error(error, path_buf.clone()))?;
        let exists = self.inner.exists(path).map_err(|error| {
            OpenWriteError::wrap_io_error(io::Error::other(error), path_buf.clone())
        })?;
        if exists {
            return Err(OpenWriteError::FileAlreadyExists(path_buf));
        }
        self.write_file_locked(&mut usage, path, &[])
            .map_err(|error| OpenWriteError::wrap_io_error(error, path_buf.clone()))?;
        drop(usage);
        let writer = BudgetedRamWrite::new(self.clone(), path_buf, writer_reservation);
        Ok(BufWriter::with_capacity(0, Box::new(writer)))
    }

    fn atomic_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        let file = self.inner.open_read(path)?;
        let read_bytes = u64::try_from(file.len()).map_err(|_| {
            OpenReadError::wrap_io_error(
                self.io_memory_error(MemoryBudgetError::AccountingOverflow),
                path.to_path_buf(),
            )
        })?;
        let mut usage = self
            .lock_usage()
            .map_err(|error| OpenReadError::wrap_io_error(error, path.to_path_buf()))?;
        self.retained_charge_locked(&mut usage, read_bytes, false)
            .map_err(|error| OpenReadError::wrap_io_error(error, path.to_path_buf()))?;
        drop(usage);
        let bytes = file
            .read_bytes()
            .map_err(|error| OpenReadError::wrap_io_error(error, path.to_path_buf()))?;
        Ok(bytes.as_slice().to_vec())
    }

    fn atomic_write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        let scratch_bytes = u64::try_from(data.len())
            .map_err(|_| self.io_memory_error(MemoryBudgetError::AccountingOverflow))?;
        let _scratch = self.scratch_reservation(scratch_bytes)?;
        let mut usage = self.lock_usage()?;
        self.write_file_locked(&mut usage, path, data)
    }

    fn sync_directory(&self) -> io::Result<()> {
        self.inner.sync_directory()
    }

    fn watch(&self, watch_callback: WatchCallback) -> tantivy::Result<WatchHandle> {
        self.inner.watch(watch_callback)
    }
}
