use maestria_domain::{DomainInput, EventId, HarnessRunId, KernelState, ScopeId};
use maestria_governance::{
    ApprovalGate, AutonomyProfile, ClassifyRisk, MemoryPromotionGate, Scope, ValidationGate,
};
use maestria_ports::{
    ApprovalRepository, ArtifactRepository, BlobStore, CardRepository, ChunkRepository,
    EffectJournal, EmbeddingProvider, EventLog, EvidenceRepository, FullTextIndex, GraphIndex,
    HarnessAdapter, IdAllocator, Parser, SearchKnowledgeExecutor, VectorIndex, WebFetcher,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

pub struct RuntimeConfig {
    pub profile: AutonomyProfile,
    pub scope: Scope,
    pub scope_id: ScopeId,
    pub input_buffer_size: usize,
    pub max_concurrent_effects: usize,
    pub default_effect_timeout: Duration,
    pub max_retries: u32,
    pub embedding_model: Option<String>,
    pub drain_effects_on_shutdown: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            profile: AutonomyProfile::TrustedWorkspace,
            scope: Scope::default(),
            scope_id: ScopeId::new(1),
            input_buffer_size: 1024,
            max_concurrent_effects: 16,
            default_effect_timeout: Duration::from_secs(300),
            max_retries: 3,
            embedding_model: None,
            drain_effects_on_shutdown: false,
        }
    }
}
pub struct Adapters {
    pub event_log: Arc<dyn EventLog + Send + Sync>,
    pub blob_store: Arc<dyn BlobStore + Send + Sync>,
    pub search_index: Arc<dyn FullTextIndex + Send + Sync>,
    pub harness: Arc<dyn HarnessAdapter + Send + Sync>,
    pub parser: Arc<dyn Parser + Send + Sync>,
    pub artifact_repo: Arc<dyn ArtifactRepository + Send + Sync>,
    pub chunk_repo: Arc<dyn ChunkRepository + Send + Sync>,
    pub card_repo: Arc<dyn CardRepository + Send + Sync>,
    pub evidence_repo: Arc<dyn EvidenceRepository + Send + Sync>,
    pub embedding_provider: Option<Arc<dyn EmbeddingProvider + Send + Sync>>,
    pub search_executor: Option<Arc<dyn SearchKnowledgeExecutor + Send + Sync>>,
    pub vector_index: Arc<dyn VectorIndex + Send + Sync>,
    pub graph_index: Arc<dyn GraphIndex + Send + Sync>,
    pub web_fetcher: Arc<dyn WebFetcher + Send + Sync>,
    pub id_allocator: Arc<dyn IdAllocator + Send + Sync>,
    pub effect_journal: Arc<dyn EffectJournal + Send + Sync>,
    pub approval_repo: Arc<dyn ApprovalRepository + Send + Sync>,
}

pub struct Governance {
    pub classifier: Arc<dyn ClassifyRisk + Send + Sync>,
    pub approval_gate: Arc<dyn ApprovalGate + Send + Sync>,
    pub validation_gate: Arc<dyn ValidationGate + Send + Sync>,
    pub memory_promotion_gate: Arc<dyn MemoryPromotionGate + Send + Sync>,
}
pub(crate) type HarnessFeedbackAcks = Arc<Mutex<BTreeMap<EventId, (HarnessRunId, u64)>>>;

/// Bundles everything an effect handler needs at execution time.
#[derive(Clone)]
pub struct EffectExecutionContext {
    pub adapters: Arc<Adapters>,
    pub governance: Arc<Governance>,
    pub profile: AutonomyProfile,
    pub scope: Scope,
    pub scope_id: ScopeId,
    pub state: Arc<RwLock<KernelState>>,
    pub input_tx: mpsc::Sender<DomainInput>,
    pub feedback_acks: HarnessFeedbackAcks,
    pub embedding_model: Option<String>,
    pub default_effect_timeout: Duration,
    pub max_retries: u32,
}

#[cfg(test)]
impl EffectExecutionContext {
    /// Convenience constructor for tests with sensible defaults.
    pub(crate) fn test_default(
        adapters: Arc<Adapters>,
        governance: Arc<Governance>,
        state: Arc<RwLock<KernelState>>,
        input_tx: mpsc::Sender<DomainInput>,
    ) -> Self {
        Self {
            adapters,
            governance,
            profile: AutonomyProfile::TrustedWorkspace,
            scope: Scope::new(vec![], vec![], vec!["shell".into()], vec![], false),
            scope_id: ScopeId::new(1),
            state,
            input_tx,
            feedback_acks: Arc::new(Mutex::new(BTreeMap::new())),
            embedding_model: None,
            default_effect_timeout: Duration::from_secs(300),
            max_retries: 3,
        }
    }
}
