use std::mem::size_of;

use sillage_ports::PortError;
use tantivy::store::Compressor;

use super::{
    BudgetedDocument, DOCSTORE_BLOCK_BYTES, DocumentMemory, checked_add, checked_mul, checked_sum,
    usize_to_u64,
};
use crate::{
    ram_directory::DirectoryCreditError,
    ram_memory::MemoryBudgetError,
    tantivy_index::{TantivyFullTextIndex, WriterMemoryUsage},
};

const ARENA_PAGE_BYTES: u64 = 1 << 20;
const HASH_TERMS_PER_DOCUMENT: u64 = 3;
// Tantivy's delete queue stores DeleteOperation in a Vec, then moves it into a
// boxed block while flushing; five slots/op cover its small initial capacity,
// geometric growth, and the temporary old-Vec/new-block overlap.
// TermWeight owns a Term, record option, BM25 state, and scoring flag.
const DELETE_OPERATION_BYTES: u64 = size_of::<(u64, *const (), *const ())>() as u64;
const DELETE_QUEUE_GROWTH_FACTOR: u64 = 5;
const DELETE_WEIGHT_OVERHEAD_BYTES: u64 = 128;
// Pre-admit retained output and separate scratch estimates, not hard encoded-size or RSS bounds.

#[derive(Default)]
struct BatchOutput {
    documents: u64,
    stored_bytes: u64,
    token_bytes: u64,
    tokens: u64,
    terms: u64,
    fields: u64,
}

impl BatchOutput {
    fn add_document(&mut self, memory: DocumentMemory) -> Result<(), MemoryBudgetError> {
        self.documents =
            checked_add(self.documents, 1).ok_or(MemoryBudgetError::AccountingOverflow)?;
        self.stored_bytes = checked_add(self.stored_bytes, memory.serialized_bytes)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        self.token_bytes = checked_add(self.token_bytes, memory.token_bytes)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        self.tokens = checked_add(self.tokens, memory.token_count)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        self.terms = checked_add(
            self.terms,
            checked_add(memory.token_count, HASH_TERMS_PER_DOCUMENT)
                .ok_or(MemoryBudgetError::AccountingOverflow)?,
        )
        .ok_or(MemoryBudgetError::AccountingOverflow)?;
        self.fields = checked_add(self.fields, memory.field_count)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        Ok(())
    }
}

impl TantivyFullTextIndex {
    pub(crate) fn reserve_writer_batch<I>(
        &self,
        documents: &[BudgetedDocument],
        delete_key_lengths: I,
    ) -> Result<(), PortError>
    where
        I: IntoIterator<Item = usize>,
    {
        let Some(writer_memory) = &self.writer_memory else {
            return Ok(());
        };
        let mut state = writer_memory.lock().map_err(|_| {
            PortError::internal("budgeted Tantivy writer", "writer memory lock poisoned")
        })?;
        let mut output = BatchOutput::default();
        let mut max_document_bytes = state.max_document_bytes;
        let mut max_stored_document_bytes = state.max_stored_document_bytes;
        for document in documents {
            let memory = document.memory;
            output
                .add_document(memory)
                .map_err(|error| self.memory_budget_error(error))?;
            max_document_bytes = max_document_bytes.max(
                writer_document_growth(&memory).map_err(|error| self.memory_budget_error(error))?,
            );
            max_stored_document_bytes = max_stored_document_bytes.max(memory.serialized_bytes);
        }
        let documents_count = checked_add(state.segment_documents, output.documents)
            .ok_or_else(|| self.accounting_overflow())?;
        let tokens = checked_add(state.segment_tokens, output.tokens)
            .ok_or_else(|| self.accounting_overflow())?;
        let terms = checked_add(state.segment_terms, output.terms)
            .ok_or_else(|| self.accounting_overflow())?;
        let historical_documents = checked_add(state.historical_documents, output.documents)
            .ok_or_else(|| self.accounting_overflow())?;

        let (delete_operations, delete_queue_bytes) =
            self.pending_delete_accounting(&state, documents, delete_key_lengths)?;
        // A queued delete may touch all historical segments; reserve only new envelope growth.
        let pending_delete_files_credit = if delete_operations == 0 {
            0
        } else {
            historical_documents
        };
        let new_delete_files =
            pending_delete_files_credit.saturating_sub(state.pending_delete_files_credit);
        let serialization_bytes = writer_serialization_growth(documents_count, tokens, terms)
            .map_err(|error| self.memory_budget_error(error))?;
        let writer_overage = max_document_bytes.saturating_sub(state.indexing_budget_bytes);
        // Reader/cache capacities are opaque; 2 KiB per document covers up to seven
        // managed paths plus a temporary hash-table resize.
        let reader_native_bytes = checked_add(
            state.reader_native_bytes,
            checked_mul(output.documents, 2_048)
                .map_err(|error| self.memory_budget_error(error))?,
        )
        .ok_or_else(|| self.accounting_overflow())?;
        let required = checked_sum(&[
            state.rehash_bytes,
            writer_overage,
            serialization_bytes,
            delete_queue_bytes,
            reader_native_bytes,
        ])
        .map_err(|error| self.memory_budget_error(error))?;
        reserve_batch_output(
            self,
            &mut state,
            required,
            &output,
            historical_documents,
            new_delete_files,
            pending_delete_files_credit,
        )?;
        state.segment_documents = documents_count;
        state.segment_tokens = tokens;
        state.segment_terms = terms;
        state.max_document_bytes = max_document_bytes;
        state.historical_documents = historical_documents;
        state.max_stored_document_bytes = max_stored_document_bytes;
        state.delete_operations = delete_operations;
        state.pending_delete_files_credit = pending_delete_files_credit;
        state.delete_queue_bytes = delete_queue_bytes;
        state.reader_native_bytes = reader_native_bytes;
        Ok(())
    }

    fn pending_delete_accounting<I>(
        &self,
        state: &WriterMemoryUsage,
        documents: &[BudgetedDocument],
        delete_key_lengths: I,
    ) -> Result<(u64, u64), PortError>
    where
        I: IntoIterator<Item = usize>,
    {
        let mut operations = state.delete_operations;
        let mut queue_bytes = state.delete_queue_bytes;
        for key_len in documents
            .iter()
            .map(|document| document.delete_key.len())
            .chain(delete_key_lengths)
        {
            let term_bytes = checked_sum(&[
                usize_to_u64(key_len).ok_or_else(|| self.accounting_overflow())?,
                u64::try_from(size_of::<tantivy::schema::Field>())
                    .map_err(|_| self.accounting_overflow())?,
            ])
            .map_err(|error| self.memory_budget_error(error))?;
            let operation_bytes = checked_sum(&[
                checked_mul(DELETE_OPERATION_BYTES, DELETE_QUEUE_GROWTH_FACTOR)
                    .map_err(|error| self.memory_budget_error(error))?,
                DELETE_WEIGHT_OVERHEAD_BYTES,
                checked_mul(term_bytes, 2).map_err(|error| self.memory_budget_error(error))?,
            ])
            .map_err(|error| self.memory_budget_error(error))?;
            operations = checked_add(operations, 1).ok_or_else(|| self.accounting_overflow())?;
            queue_bytes = checked_add(queue_bytes, operation_bytes)
                .ok_or_else(|| self.accounting_overflow())?;
        }
        Ok((operations, queue_bytes))
    }
}

fn reserve_batch_output(
    index: &TantivyFullTextIndex,
    state: &mut WriterMemoryUsage,
    writer_required: u64,
    output: &BatchOutput,
    historical_documents: u64,
    new_delete_files: u64,
    delete_path_target: u64,
) -> Result<(), PortError> {
    let (retained, scratch, managed_new_base_paths) =
        batch_output_credit(index, output, historical_documents, new_delete_files)?;
    let directory = index.ram_directory.as_ref().ok_or_else(|| {
        PortError::internal(
            "budgeted Tantivy writer",
            "RAM directory accounting is missing",
        )
    })?;
    directory
        .reserve_admission(
            &mut state.reservation,
            writer_required,
            retained,
            scratch,
            managed_new_base_paths,
            delete_path_target,
        )
        .map_err(|error| map_directory_credit_error(index, error))
}

fn batch_output_credit(
    index: &TantivyFullTextIndex,
    output: &BatchOutput,
    historical_documents: u64,
    new_delete_files: u64,
) -> Result<(u64, u64, u64), PortError> {
    let directory = index.ram_directory.as_ref().ok_or_else(|| {
        PortError::internal(
            "budgeted Tantivy writer",
            "RAM directory accounting is missing",
        )
    })?;
    let mut retained = 0;
    let mut scratch = 0;
    let mut base_file_count = 0;
    if output.documents > 0 {
        let store_bytes = stored_output_bound(
            index.index.settings().docstore_compression,
            output.stored_bytes,
        )
        .map_err(|error| index.memory_budget_error(error))?;
        let serialization_bytes =
            writer_serialization_growth(output.documents, output.tokens, output.terms)
                .map_err(|error| index.memory_budget_error(error))?;
        let segment_envelope = checked_mul(output.documents, 2 * DOCSTORE_BLOCK_BYTES + 4_096)
            .map_err(|error| index.memory_budget_error(error))?;
        let payload = checked_sum(&[
            store_bytes,
            output.token_bytes,
            checked_mul(output.fields, 8).map_err(|error| index.memory_budget_error(error))?,
            serialization_bytes,
            segment_envelope,
        ])
        .map_err(|error| index.memory_budget_error(error))?;
        base_file_count =
            checked_mul(output.documents, 6).map_err(|error| index.memory_budget_error(error))?;
        let (base_retained, base_scratch) = directory
            .estimate_file_output_credit(payload, base_file_count)
            .map_err(|error| index.memory_budget_error(error))?;
        retained =
            checked_add(retained, base_retained).ok_or_else(|| index.accounting_overflow())?;
        scratch = checked_add(scratch, base_scratch).ok_or_else(|| index.accounting_overflow())?;
    }
    if new_delete_files > 0 {
        let words = historical_documents
            .div_ceil(64)
            .checked_add(new_delete_files)
            .ok_or_else(|| index.accounting_overflow())?;
        let delete_payload = checked_sum(&[
            checked_mul(new_delete_files, 4).map_err(|error| index.memory_budget_error(error))?,
            checked_mul(words, 8).map_err(|error| index.memory_budget_error(error))?,
        ])
        .map_err(|error| index.memory_budget_error(error))?;
        let (delete_retained, delete_scratch) = directory
            .estimate_file_output_credit(delete_payload, new_delete_files)
            .map_err(|error| index.memory_budget_error(error))?;
        retained =
            checked_add(retained, delete_retained).ok_or_else(|| index.accounting_overflow())?;
        scratch =
            checked_add(scratch, delete_scratch).ok_or_else(|| index.accounting_overflow())?;
    }
    Ok((retained, scratch, base_file_count))
}

// Tantivy writes 16-KiB docstore blocks. Pinned lz4_flex adds 20 bytes and
// floor(110*n/100); Tantivy adds a four-byte length. Optional compressors use 2*n+64.
fn stored_output_bound(compressor: Compressor, bytes: u64) -> Result<u64, MemoryBudgetError> {
    if compressor == Compressor::None {
        return Ok(bytes);
    }
    let blocks = bytes.div_ceil(DOCSTORE_BLOCK_BYTES);
    let (encoded_bytes, overhead_per_block) = if compressor == Compressor::Lz4 {
        (checked_mul(bytes, 110)? / 100, 24)
    } else {
        (checked_mul(bytes, 2)?, 64)
    };
    checked_add(encoded_bytes, checked_mul(blocks, overhead_per_block)?)
        .ok_or(MemoryBudgetError::AccountingOverflow)
}

fn map_directory_credit_error(
    index: &TantivyFullTextIndex,
    error: DirectoryCreditError,
) -> PortError {
    match error {
        DirectoryCreditError::Memory(error) => index.memory_budget_error(error),
        DirectoryCreditError::Poisoned => PortError::internal(
            "budgeted RAM directory",
            "directory accounting lock poisoned",
        ),
        DirectoryCreditError::ActiveScratch => PortError::internal(
            "budgeted RAM directory",
            "active write scratch prevents admission",
        ),
    }
}

impl TantivyFullTextIndex {
    pub(crate) fn reserve_commit_growth(
        &self,
        memory: &mut WriterMemoryUsage,
    ) -> Result<(), PortError> {
        let directory = self.ram_directory.as_ref().ok_or_else(|| {
            PortError::internal(
                "budgeted Tantivy commit",
                "RAM directory accounting is missing",
            )
        })?;
        let metadata_bytes = checked_sum(&[
            4_096,
            usize_to_u64(crate::schema::CANONICAL_SCHEMA.len())
                .ok_or_else(|| self.accounting_overflow())?,
            checked_mul(memory.historical_documents, 2_048)
                .map_err(|error| self.memory_budget_error(error))?,
        ])
        .map_err(|error| self.memory_budget_error(error))?;
        let metadata_retained = directory
            .atomic_write_retained_bytes(metadata_bytes, 9)
            .map_err(|error| self.memory_budget_error(error))?;
        let handle_count = checked_add(
            checked_mul(memory.historical_documents, 14)
                .map_err(|error| self.memory_budget_error(error))?,
            1,
        )
        .ok_or_else(|| self.accounting_overflow())?;
        let handle_bytes = checked_mul(
            handle_count,
            directory
                .file_handle_bytes()
                .map_err(|error| self.memory_budget_error(error))?,
        )
        .map_err(|error| self.memory_budget_error(error))?;
        let retained = checked_sum(&[metadata_retained, metadata_bytes, handle_bytes])
            .map_err(|error| self.memory_budget_error(error))?;
        let required = checked_add(memory.reservation.reserved_bytes(), 65_536)
            .ok_or_else(|| self.accounting_overflow())?;
        let reader_native_bytes = checked_add(memory.reader_native_bytes, 65_536)
            .ok_or_else(|| self.accounting_overflow())?;
        directory
            .reserve_admission(
                &mut memory.reservation,
                required,
                retained,
                metadata_bytes,
                0,
                memory.pending_delete_files_credit,
            )
            .map_err(|error| map_directory_credit_error(self, error))?;
        memory.reader_native_bytes = reader_native_bytes;
        Ok(())
    }
}

// Opaque Tantivy 0.26 native-growth admission estimates, not physical bounds.
// Include term/posting recorders, arenas, hash slots, doc-store growth and temporary buffers.
fn writer_document_growth(document: &DocumentMemory) -> Result<u64, MemoryBudgetError> {
    let terms = checked_sum(&[document.token_count, HASH_TERMS_PER_DOCUMENT])?;
    let term_index = checked_sum(&[
        document.token_bytes,
        checked_mul(document.token_count, 30)?,
        140,
    ])?;
    let postings = checked_sum(&[checked_mul(document.token_count, 52)?, 84])?;
    let arena_growth = checked_sum(&[term_index, postings, 2 * ARENA_PAGE_BYTES])?;
    let table_growth = checked_mul(terms, 64)?;
    let document_bytes = document.serialized_bytes;
    let store_growth = checked_sum(&[checked_mul(document_bytes, 2)?, 2 * DOCSTORE_BLOCK_BYTES])?;
    let term_buffer = checked_mul(checked_sum(&[document.max_token_bytes, 4])?, 3)?;
    checked_sum(&[
        arena_growth,
        table_growth,
        store_growth,
        document.text_bytes,
        term_buffer,
        64,
        checked_mul(document.field_count, 8)?,
        checked_mul(document.field_count, 128)?,
        u64::try_from(2 * size_of::<usize>()).map_err(|_| MemoryBudgetError::AccountingOverflow)?,
    ])
}

// Segment serialization retains at most one postings buffer plus its positional
// data, term offsets, doc metadata, and the synchronous 16-KiB docstore blocks.

fn writer_serialization_growth(
    documents: u64,
    tokens: u64,
    terms: u64,
) -> Result<u64, MemoryBudgetError> {
    checked_sum(&[
        checked_mul(terms, 32)?,
        checked_mul(tokens, 30)?,
        checked_mul(documents, 70)?,
        2 * DOCSTORE_BLOCK_BYTES + 4_096,
    ])
}
