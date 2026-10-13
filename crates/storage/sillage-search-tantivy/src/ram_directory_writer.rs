use std::{
    collections::LinkedList,
    io::{self, Write},
    mem::size_of,
    path::PathBuf,
};

use tantivy::directory::{AntiCallToken, TerminatingWrite};

use super::scratch::ScratchReservation;
use super::{BudgetedRamDirectory, STAGING_CHUNK_BYTES};
use crate::ram_memory::MemoryBudgetError;

pub(super) struct StagingChunk {
    data: Box<[u8; STAGING_CHUNK_BYTES]>,
}

pub(super) struct BudgetedRamWrite {
    directory: BudgetedRamDirectory,
    path: PathBuf,
    chunks: LinkedList<StagingChunk>,
    len: usize,
    dirty: bool,
    state_reservation: ScratchReservation,
}

impl BudgetedRamWrite {
    pub(super) fn new(
        directory: BudgetedRamDirectory,
        path: PathBuf,
        state_reservation: ScratchReservation,
    ) -> Self {
        Self {
            directory,
            path,
            chunks: LinkedList::new(),
            len: 0,
            dirty: false,
            state_reservation,
        }
    }

    fn reserve_staging_chunks(&mut self, count: usize) -> io::Result<()> {
        let allocation_bytes = STAGING_CHUNK_BYTES
            .checked_add(size_of::<StagingChunk>())
            .and_then(|bytes| bytes.checked_add(2 * size_of::<usize>()))
            .ok_or_else(|| {
                self.directory
                    .io_memory_error(MemoryBudgetError::AccountingOverflow)
            })?;
        let additional_bytes = allocation_bytes
            .checked_mul(count)
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or_else(|| {
                self.directory
                    .io_memory_error(MemoryBudgetError::AccountingOverflow)
            })?;
        self.state_reservation.grow(additional_bytes)
    }

    fn allocate_chunk(&mut self) {
        self.chunks.push_back(StagingChunk {
            data: Box::new([0; STAGING_CHUNK_BYTES]),
        });
    }
}

impl Write for BudgetedRamWrite {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let end = self.len.checked_add(buffer.len()).ok_or_else(|| {
            self.directory
                .io_memory_error(MemoryBudgetError::AccountingOverflow)
        })?;
        let chunks_needed = end.div_ceil(STAGING_CHUNK_BYTES);
        let new_chunks = chunks_needed.saturating_sub(self.chunks.len());
        self.reserve_staging_chunks(new_chunks)?;

        let mut written = 0;
        let tail_offset = self.len % STAGING_CHUNK_BYTES;
        if tail_offset != 0 {
            let count = (STAGING_CHUNK_BYTES - tail_offset).min(buffer.len());
            let Some(chunk) = self.chunks.back_mut() else {
                return Err(io::Error::other("reserved RAM staging capacity is missing"));
            };
            chunk.data[tail_offset..tail_offset + count].copy_from_slice(&buffer[..count]);
            written = count;
        }

        for _ in 0..new_chunks {
            self.allocate_chunk();
        }
        let final_chunk_bytes = match end % STAGING_CHUNK_BYTES {
            0 => STAGING_CHUNK_BYTES,
            bytes => bytes,
        };
        let mut source_end = buffer.len();
        for (index, chunk) in self.chunks.iter_mut().rev().take(new_chunks).enumerate() {
            let count = if index == 0 {
                final_chunk_bytes
            } else {
                STAGING_CHUNK_BYTES
            };
            let source_start = source_end
                .checked_sub(count)
                .filter(|start| *start >= written)
                .ok_or_else(|| io::Error::other("reserved RAM staging capacity is missing"))?;
            chunk.data[..count].copy_from_slice(&buffer[source_start..source_end]);
            source_end = source_start;
        }
        if source_end != written {
            return Err(io::Error::other("reserved RAM staging capacity is missing"));
        }
        self.len = end;
        self.dirty = true;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let required_chunks = self.len.div_ceil(STAGING_CHUNK_BYTES);
        if self.chunks.len() < required_chunks {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "RAM staging chunks are incomplete",
            ));
        }
        let temporary_bytes = u64::try_from(self.len).map_err(|_| {
            self.directory
                .io_memory_error(MemoryBudgetError::AccountingOverflow)
        })?;
        let _temporary_reservation = self.directory.scratch_reservation(temporary_bytes)?;
        let mut data = Vec::with_capacity(self.len);
        let mut remaining = self.len;
        for chunk in &self.chunks {
            if remaining == 0 {
                break;
            }
            let count = remaining.min(STAGING_CHUNK_BYTES);
            data.extend_from_slice(&chunk.data[..count]);
            remaining -= count;
        }
        self.directory.atomic_write_precharged(&self.path, &data)?;
        self.dirty = false;
        Ok(())
    }
}

impl TerminatingWrite for BudgetedRamWrite {
    fn terminate_ref(&mut self, _: AntiCallToken) -> io::Result<()> {
        self.flush()
    }
}
#[cfg(test)]
mod invariant_tests {
    use super::{BudgetedRamWrite, STAGING_CHUNK_BYTES};
    use crate::{RamMemoryBudget, ram_directory::BudgetedRamDirectory};
    use std::{
        error::Error,
        io::{self, Write},
        path::Path,
        sync::Arc,
    };
    use tantivy::directory::Directory;

    #[test]
    fn incomplete_staging_refuses_flush_before_publishing() -> Result<(), Box<dyn Error>> {
        let directory = BudgetedRamDirectory::new(Arc::new(RamMemoryBudget::new(2 * 1024 * 1024)))?;
        let path = Path::new("incomplete-staging");
        let mut writer = BudgetedRamWrite::new(
            directory.clone(),
            path.to_path_buf(),
            directory.scratch_reservation(0)?,
        );
        let payload = vec![b'x'; STAGING_CHUNK_BYTES + 1];
        writer.write_all(&payload)?;
        assert!(writer.chunks.pop_back().is_some());

        let flush_result = writer.flush();
        let published = Directory::exists(&directory, path)?;
        let error = match flush_result {
            Err(error) => error,
            Ok(()) => return Err("incomplete RAM staging was accepted by flush".into()),
        };
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert!(!published, "incomplete RAM staging was published");
        Ok(())
    }
}
