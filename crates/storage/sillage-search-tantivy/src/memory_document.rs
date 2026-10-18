//! Document construction and retained reservation ownership.
//!
//! Responsibility map:
//! - `accounting`: document/token size estimates and reader/output reservations.
//! - `writer`: batch/delete charges and segment growth estimates.

#[path = "memory_document_accounting.rs"]
mod accounting;
#[path = "memory_document_writer.rs"]
mod writer;

use sillage_domain::{ArtifactId, CardId, ChunkId};
use sillage_ports::{
    IndexedCard, IndexedChunk, IndexedLexicalCard, IndexedLexicalChunk, PortError,
};
use tantivy::{TantivyDocument, Term, schema::Document};

use crate::{
    keys::{card_key, chunk_key},
    ram_memory::{MemoryBudgetError, RamMemoryReservation},
    tantivy_index::TantivyFullTextIndex,
};

const DOCSTORE_BLOCK_BYTES: u64 = 16_384;

pub(crate) struct BudgetedDocument {
    document: TantivyDocument,
    delete_field: tantivy::schema::Field,
    delete_key: String,
    memory: DocumentMemory,
    _reservation: Option<RamMemoryReservation>,
}

impl Document for BudgetedDocument {
    type Value<'a>
        = <TantivyDocument as Document>::Value<'a>
    where
        Self: 'a;
    type FieldsValuesIter<'a>
        = <TantivyDocument as Document>::FieldsValuesIter<'a>
    where
        Self: 'a;

    fn iter_fields_and_values(&self) -> Self::FieldsValuesIter<'_> {
        self.document.iter_fields_and_values()
    }
}

impl BudgetedDocument {
    pub(crate) fn take_delete_term(&mut self) -> Term {
        let key = std::mem::take(&mut self.delete_key);
        Term::from_field_text(self.delete_field, &key)
    }
}

#[derive(Clone, Copy)]
struct DocumentMemory {
    node_data_bytes: usize,
    serialized_bytes: u64,
    native_bytes: u64,
    field_count: u64,
    text_bytes: u64,
    token_bytes: u64,
    token_count: u64,
    max_token_bytes: u64,
}

impl TantivyFullTextIndex {
    pub(crate) fn chunk_document(
        &self,
        chunk: IndexedChunk,
    ) -> Result<BudgetedDocument, PortError> {
        self.build_chunk_document(
            chunk.artifact_id,
            chunk.chunk_id,
            chunk.text,
            None,
            None,
            None,
        )
    }

    pub(crate) fn lexical_chunk_document(
        &self,
        chunk: IndexedLexicalChunk,
    ) -> Result<BudgetedDocument, PortError> {
        self.build_chunk_document(
            chunk.artifact_id,
            chunk.chunk_id,
            chunk.text,
            chunk.path,
            chunk.filename,
            chunk.symbol,
        )
    }

    pub(crate) fn card_document(&self, card: IndexedCard) -> Result<BudgetedDocument, PortError> {
        let card = IndexedLexicalCard {
            artifact_id: card.artifact_id,
            card_id: card.card_id,
            title: card.title,
            body: card.body,
            path: None,
            filename: None,
            symbol: None,
        };
        self.build_card_document(&card)
    }

    pub(crate) fn lexical_card_document(
        &self,
        card: IndexedLexicalCard,
    ) -> Result<BudgetedDocument, PortError> {
        self.build_card_document(&card)
    }

    fn build_chunk_document(
        &self,
        artifact_id: ArtifactId,
        chunk_id: ChunkId,
        text: String,
        path: Option<String>,
        filename: Option<String>,
        symbol: Option<String>,
    ) -> Result<BudgetedDocument, PortError> {
        let key_len = chunk_delete_key_len(artifact_id, chunk_id);
        let mut text_values = [""; 4];
        let mut text_count = 1;
        text_values[0] = &text;
        for value in [&path, &filename, &symbol].into_iter().flatten() {
            text_values[text_count] = value;
            text_count += 1;
        }
        let memory = self.document_memory(&text_values[..text_count], key_len, 3 + text_count)?;
        let reservation = self.reserve_document_memory(memory.native_bytes)?;
        let key = chunk_key(artifact_id, chunk_id);

        let mut document = TantivyDocument::with_capacity(memory.node_data_bytes);
        document.add_text(self.fields.key, key.as_str());
        document.add_u64(self.fields.artifact_id, artifact_id.value());
        document.add_u64(self.fields.chunk_id, chunk_id.value());
        document.add_text(self.fields.text, text);
        if let Some(path) = path {
            document.add_text(self.fields.path, path);
        }
        if let Some(filename) = filename {
            document.add_text(self.fields.filename, filename);
        }
        if let Some(symbol) = symbol {
            document.add_text(self.fields.symbol, symbol);
        }
        Ok(BudgetedDocument {
            document,
            delete_field: self.fields.key,
            delete_key: key,
            memory,
            _reservation: reservation,
        })
    }

    fn build_card_document(
        &self,
        card: &IndexedLexicalCard,
    ) -> Result<BudgetedDocument, PortError> {
        let key_len = card_delete_key_len(card.artifact_id, card.card_id);
        let mut text_values = [""; 5];
        let mut text_count = 2;
        text_values[0] = &card.title;
        text_values[1] = &card.body;
        for value in [&card.path, &card.filename, &card.symbol]
            .into_iter()
            .flatten()
        {
            text_values[text_count] = value;
            text_count += 1;
        }
        let memory = self.document_memory(&text_values[..text_count], key_len, 3 + text_count)?;
        let reservation = self.reserve_document_memory(memory.native_bytes)?;
        let key = card_key(card.artifact_id, card.card_id);

        let mut document = TantivyDocument::with_capacity(memory.node_data_bytes);
        document.add_text(self.fields.card_key, key.as_str());
        document.add_u64(self.fields.card_artifact_id, card.artifact_id.value());
        document.add_u64(self.fields.card_id, card.card_id.value());
        document.add_text(self.fields.card_title, &card.title);
        document.add_text(self.fields.card_body, &card.body);
        if let Some(path) = &card.path {
            document.add_text(self.fields.card_path, path);
        }
        if let Some(filename) = &card.filename {
            document.add_text(self.fields.card_filename, filename);
        }
        if let Some(symbol) = &card.symbol {
            document.add_text(self.fields.card_symbol, symbol);
        }
        Ok(BudgetedDocument {
            document,
            delete_field: self.fields.card_key,
            delete_key: key,
            memory,
            _reservation: reservation,
        })
    }
}

fn stored_text_bytes(len: usize) -> Option<usize> {
    len.checked_add(vint_length(len)?)
}

fn vint_length(len: usize) -> Option<usize> {
    let mut value = u32::try_from(len).ok()?;
    let mut bytes = 1;
    while value >= 0x80 {
        value >>= 7;
        bytes += 1;
    }
    Some(bytes)
}

pub(crate) fn chunk_delete_key_len(artifact_id: ArtifactId, chunk_id: ChunkId) -> usize {
    decimal_len(artifact_id.value()) + 1 + decimal_len(chunk_id.value())
}

pub(crate) fn card_delete_key_len(artifact_id: ArtifactId, card_id: CardId) -> usize {
    5 + decimal_len(artifact_id.value()) + 1 + decimal_len(card_id.value())
}

fn decimal_len(mut value: u64) -> usize {
    let mut length = 1;
    while value >= 10 {
        value /= 10;
        length += 1;
    }
    length
}

fn usize_to_u64(value: usize) -> Option<u64> {
    u64::try_from(value).ok()
}

fn checked_add(left: u64, right: u64) -> Option<u64> {
    left.checked_add(right)
}

fn checked_mul(left: u64, right: u64) -> Result<u64, MemoryBudgetError> {
    left.checked_mul(right)
        .ok_or(MemoryBudgetError::AccountingOverflow)
}

fn checked_sum(values: &[u64]) -> Result<u64, MemoryBudgetError> {
    values.iter().try_fold(0_u64, |sum, value| {
        sum.checked_add(*value)
            .ok_or(MemoryBudgetError::AccountingOverflow)
    })
}
