//! The chunk corpus lane: identity, field extraction, metering, and hit
//! construction for [`IndexedLexicalChunk`], plus the chunk-facing entry
//! points. Search and index behavior is the generic pipeline in
//! [`super::lane`] (Rule 16: cross-lane behavior crosses typed functions).

use super::super::execution::saturating_u64;
use super::lane::{LexicalLane, index_lane, search_lane};
use crate::lexical::{
    ChunkField, IndexedLexicalChunk, LexicalChunkHit, LexicalHitMetadata, LexicalQuery,
};
use crate::{BoundedSearch, PortError};
use maestria_domain::{ArtifactId, ChunkId};
use std::sync::{Arc, Mutex};

pub(crate) struct ChunkLane;

impl LexicalLane for ChunkLane {
    type Id = ChunkId;
    type Field = ChunkField;
    type Record = IndexedLexicalChunk;
    type Hit = LexicalChunkHit;

    fn id(record: &Self::Record) -> Self::Id {
        record.chunk_id
    }

    fn artifact_id(record: &Self::Record) -> ArtifactId {
        record.artifact_id
    }

    fn is_id_field(field: &Self::Field) -> bool {
        matches!(field, ChunkField::Id)
    }

    fn id_key(record: &Self::Record) -> String {
        format!("{}:{}", record.artifact_id.value(), record.chunk_id.value())
    }

    fn field_value<'a>(record: &'a Self::Record, field: &Self::Field) -> Option<&'a String> {
        match field {
            ChunkField::Text => Some(&record.text),
            ChunkField::Path => record.path.as_ref(),
            ChunkField::Filename => record.filename.as_ref(),
            ChunkField::Symbol => record.symbol.as_ref(),
            ChunkField::Id => None,
        }
    }

    fn field_len(record: &Self::Record, field: &Self::Field) -> usize {
        match field {
            ChunkField::Text => record.text.len(),
            ChunkField::Path => record.path.as_ref().map_or(0, String::len),
            ChunkField::Filename => record.filename.as_ref().map_or(0, String::len),
            ChunkField::Symbol => record.symbol.as_ref().map_or(0, String::len),
            ChunkField::Id => 0,
        }
    }

    fn metered_bytes(record: &Self::Record) -> u64 {
        saturating_u64(record.text.len())
            .saturating_add(
                record
                    .path
                    .as_ref()
                    .map_or(0, |value| saturating_u64(value.len())),
            )
            .saturating_add(
                record
                    .filename
                    .as_ref()
                    .map_or(0, |value| saturating_u64(value.len())),
            )
            .saturating_add(
                record
                    .symbol
                    .as_ref()
                    .map_or(0, |value| saturating_u64(value.len())),
            )
    }

    fn build_hit(record: Self::Record, metadata: LexicalHitMetadata) -> Self::Hit {
        LexicalChunkHit {
            chunk: record,
            metadata,
        }
    }

    fn hit_score(hit: &Self::Hit) -> f32 {
        hit.metadata.raw_score
    }

    fn hit_artifact_id(hit: &Self::Hit) -> ArtifactId {
        hit.chunk.artifact_id
    }

    fn hit_item_id(hit: &Self::Hit) -> Self::Id {
        hit.chunk.chunk_id
    }

    fn set_hit_rank(hit: &mut Self::Hit, rank: u32) {
        hit.metadata.raw_rank = rank;
    }
}

pub(crate) fn index_lexical_chunks(
    lexical_chunks: &Arc<Mutex<Vec<IndexedLexicalChunk>>>,
    chunks: Vec<IndexedLexicalChunk>,
) -> Result<(), PortError> {
    index_lane::<ChunkLane>(lexical_chunks, chunks)
}

pub(crate) fn search_lexical(
    lexical_chunks: &Arc<Mutex<Vec<IndexedLexicalChunk>>>,
    query: LexicalQuery<ChunkField>,
) -> Result<BoundedSearch<LexicalChunkHit>, PortError> {
    search_lexical_filtered(lexical_chunks, query, &|_, _| Ok(true))
}

pub(crate) fn search_lexical_filtered(
    lexical_chunks: &Arc<Mutex<Vec<IndexedLexicalChunk>>>,
    query: LexicalQuery<ChunkField>,
    filter: &dyn Fn(ChunkId, ArtifactId) -> Result<bool, PortError>,
) -> Result<BoundedSearch<LexicalChunkHit>, PortError> {
    search_lane::<ChunkLane>(
        lexical_chunks,
        query,
        filter,
        "lexical search query must not be empty",
        "lexical chunk query has no fields",
        "lexical chunk result limit",
    )
}
