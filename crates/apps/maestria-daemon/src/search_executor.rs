use std::{future::Future, pin::Pin, sync::Arc};

use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EventLog, EvidenceRepository,
    FullTextIndex, GraphIndex, Parser, PortError, SearchKnowledgeExecutor, VectorIndex,
};

pub(crate) struct CoreSearchExecutor {
    pub(crate) artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub(crate) chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub(crate) cards: Arc<dyn CardRepository + Send + Sync>,
    pub(crate) evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub(crate) events: Arc<dyn EventLog + Send + Sync>,
    pub(crate) parser: Arc<dyn Parser + Send + Sync>,
    pub(crate) search_index: Arc<dyn FullTextIndex + Send + Sync>,
    pub(crate) blobs: Arc<dyn BlobStore + Send + Sync>,
    pub(crate) vector_index: Arc<dyn VectorIndex + Send + Sync>,
    pub(crate) graph_index: Arc<dyn GraphIndex + Send + Sync>,
    pub(crate) retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
}

impl SearchKnowledgeExecutor for CoreSearchExecutor {
    fn search(
        &self,
        plan: maestria_domain::SearchPlan,
    ) -> Pin<Box<dyn Future<Output = Result<maestria_domain::SearchOutcome, PortError>> + Send + '_>>
    {
        let artifacts = self.artifacts.clone();
        let chunks = self.chunks.clone();
        let cards = self.cards.clone();
        let evidence = self.evidence.clone();
        let events = self.events.clone();
        let parser = self.parser.clone();
        let search_index = self.search_index.clone();
        let blobs = self.blobs.clone();
        let vector_index = self.vector_index.clone();
        let graph_index = self.graph_index.clone();
        let retrieval_policy = self.retrieval_policy.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let ports = maestria_core::CorePorts {
                    artifacts: artifacts.as_ref(),
                    chunks: chunks.as_ref(),
                    cards: cards.as_ref(),
                    evidence: evidence.as_ref(),
                    events: events.as_ref(),
                    parser: parser.as_ref(),
                    search_index: search_index.as_ref(),
                    blobs: blobs.as_ref(),
                    vector_index: Some(vector_index.as_ref()),
                    graph_index: Some(graph_index.as_ref()),
                };
                maestria_core::CoreServices::new(ports)
                    .with_retrieval_policy(retrieval_policy)
                    .search_knowledge(plan)
                    .map_err(|error| PortError::Internal {
                        message: error.to_string(),
                    })
            })
            .await
            .map_err(|error| PortError::Internal {
                message: format!("search worker failed: {error}"),
            })?
        })
    }
}
