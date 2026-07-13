use crate::error::CoreResult;
use crate::retrieval::{open_chunk_evidence, open_evidence, search};
use crate::types::{
    OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput, SearchInput, SearchOutput,
};

use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EventLog, FullTextIndex, Parser,
};

pub struct CorePorts<'a> {
    pub artifacts: &'a dyn ArtifactRepository,
    pub chunks: &'a dyn ChunkRepository,
    pub cards: &'a dyn CardRepository,
    pub evidence: &'a dyn maestria_ports::EvidenceRepository,
    pub events: &'a dyn EventLog,
    pub parser: &'a dyn Parser,
    pub search_index: &'a dyn FullTextIndex,
    pub blobs: &'a dyn BlobStore,
    pub vector_index: Option<&'a dyn maestria_ports::VectorIndex>,
}

pub struct CoreServices<'a> {
    ports: CorePorts<'a>,
}

impl<'a> CoreServices<'a> {
    pub fn new(ports: CorePorts<'a>) -> Self {
        Self { ports }
    }

    pub fn search(&self, input: SearchInput) -> CoreResult<SearchOutput> {
        search(&self.ports, input, None)
    }

    pub fn search_with_vector(
        &self,
        input: SearchInput,
        vector_query: maestria_ports::VectorSearchQuery,
    ) -> CoreResult<SearchOutput> {
        search(&self.ports, input, Some(vector_query))
    }
    pub fn open_evidence(&self, input: OpenEvidenceInput) -> CoreResult<OpenEvidenceOutput> {
        open_evidence(&self.ports, input)
    }

    pub fn open_chunk_evidence(
        &self,
        input: OpenChunkEvidenceInput,
    ) -> CoreResult<OpenEvidenceOutput> {
        open_chunk_evidence(&self.ports, input)
    }
}
