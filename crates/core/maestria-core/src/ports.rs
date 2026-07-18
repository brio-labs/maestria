use crate::error::CoreResult;
use crate::evidence_opening::{open_chunk_evidence, open_evidence};
use crate::types::{OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput};
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
    pub graph_index: Option<&'a dyn maestria_ports::GraphIndex>,
}

pub struct CoreServices<'a> {
    ports: CorePorts<'a>,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
}

impl<'a> CoreServices<'a> {
    pub fn new(ports: CorePorts<'a>) -> Self {
        Self {
            ports,
            retrieval_policy: maestria_governance::RetrievalSecurityPolicy::default(),
        }
    }

    pub fn with_retrieval_policy(
        mut self,
        policy: maestria_governance::RetrievalSecurityPolicy,
    ) -> Self {
        self.retrieval_policy = policy;
        self
    }

    pub fn open_evidence(&self, input: OpenEvidenceInput) -> CoreResult<OpenEvidenceOutput> {
        open_evidence(&self.ports, input, &self.retrieval_policy)
    }

    pub fn open_chunk_evidence(
        &self,
        input: OpenChunkEvidenceInput,
    ) -> CoreResult<OpenEvidenceOutput> {
        open_chunk_evidence(&self.ports, input, &self.retrieval_policy)
    }
}
