#![forbid(unsafe_code)]

//! Tantivy-backed full-text search projection for Maestria.
//!
//! This crate stores only rebuildable indexed chunks. Artifact metadata and blob
//! contents remain owned by their source repositories.

use std::{fs, path::Path, sync::Mutex};

use maestria_domain::{ArtifactId, CardId, ChunkId};
use maestria_ports::{
    CardHit, FullTextIndex, IndexedCard, IndexedChunk, PortError, SearchHit, SearchQuery,
};
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
    collector::TopDocs,
    doc,
    query::QueryParser,
    schema::{
        FAST, Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
        Value,
    },
};

const WRITER_MEMORY_BUDGET_BYTES: usize = 50_000_000;
const FIELD_KEY: &str = "chunk_key";
const FIELD_ARTIFACT_ID: &str = "artifact_id";
const FIELD_CHUNK_ID: &str = "chunk_id";
const FIELD_TEXT: &str = "text";
const FIELD_CARD_KEY: &str = "card_key";
const FIELD_CARD_ARTIFACT_ID: &str = "card_artifact_id";
const FIELD_CARD_ID: &str = "card_id";
const FIELD_CARD_TITLE: &str = "card_title";
const FIELD_CARD_BODY: &str = "card_body";

/// Tantivy implementation of the [`FullTextIndex`] projection port.
pub struct TantivyFullTextIndex {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    fields: IndexFields,
}

struct IndexFields {
    key: Field,
    artifact_id: Field,
    chunk_id: Field,
    text: Field,
    card_key: Field,
    card_artifact_id: Field,
    card_id: Field,
    card_title: Field,
    card_body: Field,
}

impl TantivyFullTextIndex {
    /// Create a short-lived in-memory full-text projection.
    pub fn in_memory() -> Result<Self, PortError> {
        Self::from_index(Index::create_in_ram(schema()))
    }

    /// Open or create a directory-backed full-text projection.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PortError> {
        let path = path.as_ref();
        fs::create_dir_all(path).map_err(to_io_port_error)?;
        let index = if path.join("meta.json").exists() {
            Index::open_in_dir(path).map_err(to_port_error)?
        } else {
            Index::create_in_dir(path, schema()).map_err(to_port_error)?
        };
        Self::from_index(index)
    }

    fn from_index(index: Index) -> Result<Self, PortError> {
        let fields = load_fields(index.schema())?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(to_port_error)?;
        let writer = index
            .writer_with_num_threads(1, WRITER_MEMORY_BUDGET_BYTES)
            .map_err(to_port_error)?;

        Ok(Self {
            index,
            reader,
            writer: Mutex::new(writer),
            fields,
        })
    }

    fn chunk_document(&self, chunk: &IndexedChunk) -> TantivyDocument {
        doc!(
            self.fields.key => chunk_key(chunk.artifact_id, chunk.chunk_id),
            self.fields.artifact_id => chunk.artifact_id.value(),
            self.fields.chunk_id => chunk.chunk_id.value(),
            self.fields.text => chunk.text.clone(),
        )
    }

    fn read_chunk(&self, document: &TantivyDocument) -> Result<IndexedChunk, PortError> {
        let artifact_id = document
            .get_first(self.fields.artifact_id)
            .and_then(|value| value.as_u64())
            .map(ArtifactId::new)
            .ok_or_else(|| PortError::Internal {
                message: "indexed chunk is missing artifact id".to_string(),
            })?;
        let chunk_id = document
            .get_first(self.fields.chunk_id)
            .and_then(|value| value.as_u64())
            .map(ChunkId::new)
            .ok_or_else(|| PortError::Internal {
                message: "indexed chunk is missing chunk id".to_string(),
            })?;
        let text = document
            .get_first(self.fields.text)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| PortError::Internal {
                message: "indexed chunk is missing text".to_string(),
            })?;

        Ok(IndexedChunk {
            artifact_id,
            chunk_id,
            text,
        })
    }

    fn card_document(&self, card: &IndexedCard) -> TantivyDocument {
        doc!(
            self.fields.card_key => card_key(card.artifact_id, card.card_id),
            self.fields.card_artifact_id => card.artifact_id.value(),
            self.fields.card_id => card.card_id.value(),
            self.fields.card_title => card.title.clone(),
            self.fields.card_body => card.body.clone(),
        )
    }

    fn read_card(&self, document: &TantivyDocument) -> Result<IndexedCard, PortError> {
        let artifact_id = document
            .get_first(self.fields.card_artifact_id)
            .and_then(|value| value.as_u64())
            .map(ArtifactId::new)
            .ok_or_else(|| PortError::Internal {
                message: "indexed card is missing artifact id".to_string(),
            })?;
        let card_id = document
            .get_first(self.fields.card_id)
            .and_then(|value| value.as_u64())
            .map(CardId::new)
            .ok_or_else(|| PortError::Internal {
                message: "indexed card is missing card id".to_string(),
            })?;
        let title = document
            .get_first(self.fields.card_title)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| PortError::Internal {
                message: "indexed card is missing title".to_string(),
            })?;
        let body = document
            .get_first(self.fields.card_body)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| PortError::Internal {
                message: "indexed card is missing body".to_string(),
            })?;

        Ok(IndexedCard {
            artifact_id,
            card_id,
            title,
            body,
        })
    }
}

impl FullTextIndex for TantivyFullTextIndex {
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        let mut writer = self.writer.lock().map_err(|_| PortError::Internal {
            message: "tantivy writer lock poisoned".to_string(),
        })?;

        for chunk in chunks {
            writer.delete_term(Term::from_field_text(
                self.fields.key,
                &chunk_key(chunk.artifact_id, chunk.chunk_id),
            ));
            writer
                .add_document(self.chunk_document(&chunk))
                .map_err(to_port_error)?;
        }

        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)
    }

    fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInput {
                message: "search query must not be empty".to_string(),
            });
        }
        if query.limit == 0 {
            return Ok(Vec::new());
        }

        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(&self.index, vec![self.fields.text]);
        let parsed_query =
            parser
                .parse_query(trimmed)
                .map_err(|error| PortError::InvalidInput {
                    message: format!("invalid search query: {error}"),
                })?;
        let top_docs = searcher
            .search(
                &parsed_query,
                &TopDocs::with_limit(query.limit).order_by_score(),
            )
            .map_err(to_port_error)?;
        let mut hits = Vec::with_capacity(top_docs.len());

        for (score, address) in top_docs {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            hits.push(SearchHit {
                chunk: self.read_chunk(&document)?,
                score: score_to_u32(score),
            });
        }

        Ok(hits)
    }

    fn index_cards(&self, cards: Vec<IndexedCard>) -> Result<(), PortError> {
        let mut writer = self.writer.lock().map_err(|_| PortError::Internal {
            message: "tantivy writer lock poisoned".to_string(),
        })?;

        for card in cards {
            writer.delete_term(Term::from_field_text(
                self.fields.card_key,
                &card_key(card.artifact_id, card.card_id),
            ));
            writer
                .add_document(self.card_document(&card))
                .map_err(to_port_error)?;
        }

        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)
    }

    fn search_cards(&self, query: SearchQuery) -> Result<Vec<CardHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInput {
                message: "search query must not be empty".to_string(),
            });
        }
        if query.limit == 0 {
            return Ok(Vec::new());
        }

        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.card_title, self.fields.card_body],
        );
        let parsed_query =
            parser
                .parse_query(trimmed)
                .map_err(|error| PortError::InvalidInput {
                    message: format!("invalid search query: {error}"),
                })?;
        let top_docs = searcher
            .search(
                &parsed_query,
                &TopDocs::with_limit(query.limit).order_by_score(),
            )
            .map_err(to_port_error)?;

        let mut scored: Vec<(u32, String, IndexedCard)> = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            let doc_key = document
                .get_first(self.fields.card_key)
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .ok_or_else(|| PortError::Internal {
                    message: "indexed card is missing card key".to_string(),
                })?;
            scored.push((score_to_u32(score), doc_key, self.read_card(&document)?));
        }

        // Deterministic ordering: descending score, ascending key for ties.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

        Ok(scored
            .into_iter()
            .map(|(score, _key, card)| CardHit { card, score })
            .collect())
    }
}

fn schema() -> Schema {
    let mut builder = Schema::builder();
    let text_indexing = TextFieldIndexing::default()
        .set_tokenizer("default")
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let text_options = TextOptions::default()
        .set_indexing_options(text_indexing.clone())
        .set_stored();

    builder.add_text_field(FIELD_KEY, STRING | STORED);
    builder.add_u64_field(FIELD_ARTIFACT_ID, FAST | STORED);
    builder.add_u64_field(FIELD_CHUNK_ID, FAST | STORED);
    builder.add_text_field(FIELD_TEXT, text_options.clone());
    builder.add_text_field(FIELD_CARD_KEY, STRING | STORED);
    builder.add_u64_field(FIELD_CARD_ARTIFACT_ID, FAST | STORED);
    builder.add_u64_field(FIELD_CARD_ID, FAST | STORED);
    builder.add_text_field(FIELD_CARD_TITLE, text_options.clone());
    builder.add_text_field(FIELD_CARD_BODY, text_options);
    builder.build()
}

fn load_fields(schema: Schema) -> Result<IndexFields, PortError> {
    Ok(IndexFields {
        key: schema_field(&schema, FIELD_KEY)?,
        artifact_id: schema_field(&schema, FIELD_ARTIFACT_ID)?,
        chunk_id: schema_field(&schema, FIELD_CHUNK_ID)?,
        text: schema_field(&schema, FIELD_TEXT)?,
        card_key: schema_field(&schema, FIELD_CARD_KEY)?,
        card_artifact_id: schema_field(&schema, FIELD_CARD_ARTIFACT_ID)?,
        card_id: schema_field(&schema, FIELD_CARD_ID)?,
        card_title: schema_field(&schema, FIELD_CARD_TITLE)?,
        card_body: schema_field(&schema, FIELD_CARD_BODY)?,
    })
}

fn schema_field(schema: &Schema, name: &str) -> Result<Field, PortError> {
    schema.get_field(name).map_err(|_| PortError::Internal {
        message: format!("tantivy schema is missing {name} field"),
    })
}

fn chunk_key(artifact_id: ArtifactId, chunk_id: ChunkId) -> String {
    format!("{}:{}", artifact_id.value(), chunk_id.value())
}

fn card_key(artifact_id: ArtifactId, card_id: CardId) -> String {
    format!("card:{}:{}", artifact_id.value(), card_id.value())
}

fn score_to_u32(score: f32) -> u32 {
    if !score.is_finite() || score <= 0.0 {
        return 0;
    }

    let scaled = score * 1_000.0;
    if scaled >= u32::MAX as f32 {
        u32::MAX
    } else {
        scaled.round() as u32
    }
}

fn to_port_error(error: tantivy::TantivyError) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
    }
}

fn to_io_port_error(error: std::io::Error) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod card_tests;
#[cfg(test)]
mod tests;
