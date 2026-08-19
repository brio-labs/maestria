//! The chunk corpus lane: identity for [`IndexedLexicalChunk`], plus the
//! chunk-facing index entry point. The typed lexical *search* family was
//! removed with ADR-0005 (expiry v0.7.0); index behavior is the generic
//! pipeline in [`super::lane`] (Rule 16: cross-lane behavior crosses typed
//! functions).

use super::lane::{LexicalLane, index_lane};
use crate::PortError;
use crate::lexical::IndexedLexicalChunk;
use maestria_domain::{ArtifactId, ChunkId};
use std::sync::{Arc, Mutex};

pub(crate) struct ChunkLane;

impl LexicalLane for ChunkLane {
    type Id = ChunkId;
    type Record = IndexedLexicalChunk;

    fn id(record: &Self::Record) -> Self::Id {
        record.chunk_id
    }

    fn artifact_id(record: &Self::Record) -> ArtifactId {
        record.artifact_id
    }
}

pub(crate) fn index_lexical_chunks(
    lexical_chunks: &Arc<Mutex<Vec<IndexedLexicalChunk>>>,
    chunks: Vec<IndexedLexicalChunk>,
) -> Result<(), PortError> {
    index_lane::<ChunkLane>(lexical_chunks, chunks)
}
