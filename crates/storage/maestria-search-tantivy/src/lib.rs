#![forbid(unsafe_code)]

//! Tantivy-backed full-text search projection for Maestria.
//!
//! This crate stores only rebuildable indexed chunks. Artifact metadata and blob
//! contents remain owned by their source repositories.

use std::{fs, path::Path, sync::Mutex};

use maestria_domain::{ArtifactId, ChunkId};
use maestria_ports::{FullTextIndex, IndexedChunk, PortError, SearchHit, SearchQuery};
use tantivy::{
    collector::TopDocs,
    doc,
    query::QueryParser,
    schema::{
        Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value, FAST, STORED,
        STRING,
    },
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
};

const WRITER_MEMORY_BUDGET_BYTES: usize = 50_000_000;
const FIELD_KEY: &str = "chunk_key";
const FIELD_ARTIFACT_ID: &str = "artifact_id";
const FIELD_CHUNK_ID: &str = "chunk_id";
const FIELD_TEXT: &str = "text";

/// Tantivy implementation of the [`FullTextIndex`] projection port.
pub struct TantivyFullTextIndex {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    fields: IndexFields,
}

#[derive(Clone, Copy)]
struct IndexFields {
    key: Field,
    artifact_id: Field,
    chunk_id: Field,
    text: Field,
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
            .writer(WRITER_MEMORY_BUDGET_BYTES)
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
            .search(&parsed_query, &TopDocs::with_limit(query.limit))
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
}

fn schema() -> Schema {
    let mut builder = Schema::builder();
    let text_indexing = TextFieldIndexing::default()
        .set_tokenizer("default")
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let text_options = TextOptions::default()
        .set_indexing_options(text_indexing)
        .set_stored();

    builder.add_text_field(FIELD_KEY, STRING | STORED);
    builder.add_u64_field(FIELD_ARTIFACT_ID, FAST | STORED);
    builder.add_u64_field(FIELD_CHUNK_ID, FAST | STORED);
    builder.add_text_field(FIELD_TEXT, text_options);
    builder.build()
}

fn load_fields(schema: Schema) -> Result<IndexFields, PortError> {
    Ok(IndexFields {
        key: schema_field(&schema, FIELD_KEY)?,
        artifact_id: schema_field(&schema, FIELD_ARTIFACT_ID)?,
        chunk_id: schema_field(&schema, FIELD_CHUNK_ID)?,
        text: schema_field(&schema, FIELD_TEXT)?,
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
mod tests {
    use super::*;

    use maestria_ports::SearchQuery;
    use tempfile::TempDir;

    fn chunk(artifact_id: u64, chunk_id: u64, text: &str) -> IndexedChunk {
        IndexedChunk {
            artifact_id: ArtifactId::new(artifact_id),
            chunk_id: ChunkId::new(chunk_id),
            text: text.to_string(),
        }
    }

    #[test]
    fn index_search_returns_source_openable_chunk_metadata() {
        let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

        index
            .index_chunks(vec![
                chunk(7, 70, "alpha source chunk"),
                chunk(8, 80, "beta unrelated chunk"),
            ])
            .expect("index chunks");

        let hits = index
            .search(SearchQuery {
                q: "alpha".to_string(),
                limit: 10,
            })
            .expect("search chunks");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk.artifact_id, ArtifactId::new(7));
        assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(70));
        assert_eq!(hits[0].chunk.text, "alpha source chunk");
        assert!(hits[0].score > 0);
    }

    #[test]
    fn limit_is_honored() {
        let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

        index
            .index_chunks(vec![
                chunk(1, 10, "shared term one"),
                chunk(1, 11, "shared term two"),
                chunk(1, 12, "shared term three"),
            ])
            .expect("index chunks");

        let hits = index
            .search(SearchQuery {
                q: "shared".to_string(),
                limit: 2,
            })
            .expect("search chunks");

        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn empty_query_is_invalid() {
        let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

        let result = index.search(SearchQuery {
            q: "  \t  ".to_string(),
            limit: 10,
        });

        assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    }

    #[test]
    fn reindexing_same_chunk_replaces_without_duplicate_hits() {
        let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

        index
            .index_chunks(vec![chunk(2, 20, "original searchable text")])
            .expect("index original chunk");
        index
            .index_chunks(vec![chunk(2, 20, "updated searchable text")])
            .expect("reindex chunk");

        let hits = index
            .search(SearchQuery {
                q: "searchable".to_string(),
                limit: 10,
            })
            .expect("search chunks");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk.artifact_id, ArtifactId::new(2));
        assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(20));
        assert_eq!(hits[0].chunk.text, "updated searchable text");
    }

    #[test]
    fn no_results_for_missing_term() {
        let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

        index
            .index_chunks(vec![chunk(3, 30, "present words only")])
            .expect("index chunks");

        let hits = index
            .search(SearchQuery {
                q: "absent".to_string(),
                limit: 10,
            })
            .expect("search chunks");

        assert!(hits.is_empty());
    }

    #[test]
    fn directory_backed_index_can_be_reopened() {
        let directory = TempDir::new().expect("create temp directory");
        let index = TantivyFullTextIndex::open(directory.path()).expect("open directory index");
        index
            .index_chunks(vec![chunk(4, 40, "durable indexed text")])
            .expect("index chunk");
        drop(index);

        let reopened =
            TantivyFullTextIndex::open(directory.path()).expect("reopen directory index");
        let hits = reopened
            .search(SearchQuery {
                q: "durable".to_string(),
                limit: 10,
            })
            .expect("search chunks");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk.artifact_id, ArtifactId::new(4));
        assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(40));
    }
}
