pub use crate::config::EffectExecutionContext;

impl SillageRuntime {
    pub(crate) fn test_with_pre_failed_effect_task(mut self) -> Self {
        self.test_pre_failed_effect_task = true;
        self
    }

    pub(crate) async fn test_execute_effect(
        effect: SillageEffect,
        context: EffectExecutionContext,
        persistence_barrier_timeout: Option<std::time::Duration>,
    ) -> bool {
        context
            .execute_effect(effect, persistence_barrier_timeout)
            .await
            .is_ok()
    }
}
pub use crate::config::{Adapters, Governance, RuntimeConfig};
pub use crate::runtime::{
    DomainApplicationResult, FeedbackError, RuntimeHandle, RuntimeSubmissionError, SillageRuntime,
};
pub use sillage_domain::{
    DomainEvent, DomainEventEnvelope, DomainInput, KernelState, SillageEffect, ValidationReportId,
    content_hash, evidence_id_for,
};
pub use sillage_governance::{AutonomyProfile, Scope};
pub use sillage_ports::{
    HarnessRequest, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
    InMemoryChunkRepository, InMemoryEffectJournal, InMemoryEventLog, InMemoryEvidenceRepository,
    InMemoryFullTextIndex, InMemoryGraphIndex, InMemoryHarnessAdapter, InMemoryParser,
    InMemoryVectorIndex, InMemoryWebFetcher, IndexedCard, IndexedChunk, ParseContext, Parser,
    PortError, SourceSpan, VectorEmbedding, VectorIndex, WebFetcher,
};
pub use std::sync::Arc;
pub use std::time::Duration;
pub use tokio::sync::{RwLock, mpsc};
