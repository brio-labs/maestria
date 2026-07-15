use super::test_support::*;
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier, DefaultValidationGate};
use maestria_ports::{
    InMemoryApprovalRepository, InMemoryArtifactRepository, InMemoryBlobStore,
    InMemoryCardRepository, InMemoryChunkRepository, InMemoryEffectJournal, InMemoryEventLog,
    InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryGraphIndex, InMemoryHarnessAdapter,
    InMemoryIdAllocator, InMemoryParser, InMemoryVectorIndex, InMemoryWebFetcher,
};
use std::sync::Arc;

/// Returns a default set of adapters for tests, all backed by InMemory implementations.
/// Tests override specific fields with struct update syntax:
///
/// ```ignore
/// let adapters = Adapters { event_log: my_log.clone(), ..test_helpers::test_adapters() };
/// ```
pub fn test_adapters() -> Adapters {
    Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        embedding_provider: None,
        search_executor: None,
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
        id_allocator: Arc::new(InMemoryIdAllocator::new()),
        effect_journal: Arc::new(InMemoryEffectJournal::default()),
        approval_repo: Arc::new(InMemoryApprovalRepository::new()),
    }
}

/// Returns default governance backed by permissive defaults suitable for tests.
pub fn test_governance() -> Governance {
    Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)),
    }
}
