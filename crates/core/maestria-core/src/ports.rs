use crate::error::CoreResult;
use crate::ingestion::ingest_file_from_bytes;
use crate::retrieval::{open_chunk_evidence, open_evidence, search};
use crate::types::{
    IngestFileInput, IngestFileOutput, OpenChunkEvidenceInput, OpenEvidenceInput,
    OpenEvidenceOutput, SearchInput, SearchOutput,
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
}

pub struct CoreServices<'a> {
    ports: CorePorts<'a>,
}

impl<'a> CoreServices<'a> {
    pub fn new(ports: CorePorts<'a>) -> Self {
        Self { ports }
    }

    pub fn ingest_file_from_bytes(&self, input: IngestFileInput) -> CoreResult<IngestFileOutput> {
        ingest_file_from_bytes(&self.ports, input)
    }

    pub fn search(&self, input: SearchInput) -> CoreResult<SearchOutput> {
        search(&self.ports, input)
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
