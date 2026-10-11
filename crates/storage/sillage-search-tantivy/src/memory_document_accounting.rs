use std::mem::size_of;

use sillage_ports::PortError;
use tantivy::{TantivyDocument, tokenizer::TokenStream};

use super::{
    BudgetedDocument, DOCSTORE_BLOCK_BYTES, DocumentMemory, checked_add, checked_mul, checked_sum,
    stored_text_bytes, usize_to_u64,
};
use crate::{
    ram_memory::{MemoryBudgetError, RamMemoryReservation},
    tantivy_index::TantivyFullTextIndex,
};

const MAX_TOKEN_LENGTH: usize = u16::MAX as usize - 5;
const MAX_STORED_FIELDS: u64 = 8;

impl TantivyFullTextIndex {
    pub(super) fn document_memory(
        &self,
        text_values: &[&str],
        key_len: usize,
        field_count: usize,
    ) -> Result<DocumentMemory, PortError> {
        let mut node_data_bytes = stored_text_bytes(key_len)
            .and_then(|bytes| bytes.checked_add(2 * size_of::<u64>()))
            .ok_or_else(|| self.accounting_overflow())?;
        let mut text_bytes = 0_u64;
        for text in text_values {
            node_data_bytes = node_data_bytes
                .checked_add(
                    stored_text_bytes(text.len()).ok_or_else(|| self.accounting_overflow())?,
                )
                .ok_or_else(|| self.accounting_overflow())?;
            text_bytes = text_bytes
                .checked_add(usize_to_u64(text.len()).ok_or_else(|| self.accounting_overflow())?)
                .ok_or_else(|| self.accounting_overflow())?;
        }
        let slots = if field_count <= 4 {
            4
        } else {
            let capacity = field_count
                .checked_next_power_of_two()
                .ok_or_else(|| self.accounting_overflow())?;
            capacity
                .checked_add(capacity / 2)
                .ok_or_else(|| self.accounting_overflow())?
        };
        let field_count = usize_to_u64(field_count).ok_or_else(|| self.accounting_overflow())?;
        let serialized_bytes = usize_to_u64(node_data_bytes)
            .and_then(|bytes| bytes.checked_add(field_count.checked_mul(16)?))
            .ok_or_else(|| self.accounting_overflow())?;
        let slot_bytes = size_of::<tantivy::schema::Field>() + 2 * size_of::<u64>();
        let native_bytes = usize_to_u64(node_data_bytes)
            .and_then(|bytes| bytes.checked_add(usize_to_u64(slots.checked_mul(slot_bytes)?)?))
            .and_then(|bytes| bytes.checked_add(usize_to_u64(key_len.checked_mul(2)?)?))
            .and_then(|bytes| bytes.checked_add(usize_to_u64(4 * size_of::<BudgetedDocument>())?))
            .ok_or_else(|| self.accounting_overflow())?;
        let mut memory = DocumentMemory {
            node_data_bytes,
            serialized_bytes,
            native_bytes,
            field_count,
            text_bytes,
            token_bytes: 0,
            token_count: 0,
            max_token_bytes: 0,
        };
        if let Some(budget) = &self.memory_budget {
            // Native analyzer clone/token buffers and Unicode lowercase expansion.
            let tokenizer_bytes = checked_mul(text_bytes, 3)
                .and_then(|bytes| checked_sum(&[bytes, 4096]))
                .map_err(|error| self.memory_budget_error(error))?;
            let _tokenizer_reservation = budget
                .reserve(tokenizer_bytes)
                .map_err(|error| self.memory_budget_error(error))?;
            let mut analyzer = self.index.tokenizers().get("default").ok_or_else(|| {
                PortError::internal("budgeted Tantivy document", "default tokenizer is missing")
            })?;
            for text in text_values {
                let mut stream = analyzer.token_stream(text);
                while stream.advance() {
                    let token_len = stream.token().text.len();
                    if token_len > MAX_TOKEN_LENGTH {
                        continue;
                    }
                    memory.token_count = checked_add(memory.token_count, 1)
                        .ok_or_else(|| self.accounting_overflow())?;
                    memory.token_bytes = checked_add(
                        memory.token_bytes,
                        usize_to_u64(token_len).ok_or_else(|| self.accounting_overflow())?,
                    )
                    .ok_or_else(|| self.accounting_overflow())?;
                    memory.max_token_bytes = memory
                        .max_token_bytes
                        .max(usize_to_u64(token_len).ok_or_else(|| self.accounting_overflow())?);
                }
            }
        }
        Ok(memory)
    }

    pub(crate) fn reserve_document_memory(
        &self,
        bytes: u64,
    ) -> Result<Option<RamMemoryReservation>, PortError> {
        self.memory_budget
            .as_ref()
            .map(|budget| {
                budget
                    .reserve(bytes)
                    .map_err(|error| self.memory_budget_error(error))
            })
            .transpose()
    }

    pub(crate) fn reserve_reader_document(
        &self,
    ) -> Result<Option<RamMemoryReservation>, PortError> {
        let Some(budget) = &self.memory_budget else {
            return Ok(None);
        };
        let max_document_bytes = self.max_stored_document_bytes()?;
        let block_bytes = checked_sum(&[DOCSTORE_BLOCK_BYTES, max_document_bytes])
            .map_err(|error| self.memory_budget_error(error))?;
        let field_slot_bytes = size_of::<tantivy::schema::Field>() + 2 * size_of::<u64>();
        let decoded_bytes = checked_sum(&[
            // CompactDoc decoding grows node storage and temporarily owns each
            // decoded value; include reallocation overlap before asking Tantivy to decode.
            checked_mul(max_document_bytes, 4).map_err(|error| self.memory_budget_error(error))?,
            usize_to_u64(size_of::<TantivyDocument>()).ok_or_else(|| self.accounting_overflow())?,
            checked_mul(
                MAX_STORED_FIELDS * 2,
                usize_to_u64(field_slot_bytes).ok_or_else(|| self.accounting_overflow())?,
            )
            .map_err(|error| self.memory_budget_error(error))?,
        ])
        .map_err(|error| self.memory_budget_error(error))?;
        budget
            .reserve(
                checked_sum(&[block_bytes, decoded_bytes])
                    .map_err(|error| self.memory_budget_error(error))?,
            )
            .map(Some)
            .map_err(|error| self.memory_budget_error(error))
    }

    pub(crate) fn search_output_bound(&self) -> Result<u64, PortError> {
        let max_document_bytes = self.max_stored_document_bytes()?;
        checked_sum(&[
            checked_mul(max_document_bytes, 2).map_err(|error| self.memory_budget_error(error))?,
            512,
        ])
        .map_err(|error| self.memory_budget_error(error))
    }

    pub(crate) fn reserve_search_output(
        &self,
        bytes: u64,
    ) -> Result<Option<RamMemoryReservation>, PortError> {
        self.reserve_document_memory(bytes)
    }

    fn max_stored_document_bytes(&self) -> Result<u64, PortError> {
        let Some(writer_memory) = &self.writer_memory else {
            return Ok(0);
        };
        writer_memory
            .lock()
            .map(|usage| usage.max_stored_document_bytes)
            .map_err(|_| {
                PortError::internal("budgeted Tantivy reader", "writer memory lock poisoned")
            })
    }

    pub(crate) fn memory_budget_error(&self, error: MemoryBudgetError) -> PortError {
        if let Some(directory) = &self.ram_directory {
            directory.remember_memory_error(error);
        }
        PortError::downstream("budgeted in-memory full-text index", error.to_string())
    }

    pub(crate) fn accounting_overflow(&self) -> PortError {
        self.memory_budget_error(MemoryBudgetError::AccountingOverflow)
    }
}
