pub use crate::config::EffectExecutionContext;

impl MaestriaRuntime {
    pub(crate) fn test_with_pre_failed_effect_task(mut self) -> Self {
        self.test_pre_failed_effect_task = true;
        self
    }

    pub(crate) async fn test_execute_effect(
        effect: MaestriaEffect,
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
    DomainApplicationResult, FeedbackError, MaestriaRuntime, RuntimeHandle, RuntimeSubmissionError,
};
pub use maestria_domain::{
    DomainEvent, DomainEventEnvelope, DomainInput, KernelState, MaestriaEffect, ValidationReportId,
    content_hash, evidence_id_for,
};
pub use maestria_governance::{AutonomyProfile, Scope};
pub use maestria_ports::{
    HarnessRequest, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
    InMemoryChunkRepository, InMemoryEffectJournal, InMemoryEventLog, InMemoryEvidenceRepository,
    InMemoryFullTextIndex, InMemoryGraphIndex, InMemoryHarnessAdapter, InMemoryParser,
    InMemoryVectorIndex, InMemoryWebFetcher, IndexedCard, IndexedChunk, ParseContext, Parser,
    PortError, SourceSpan, VectorEmbedding, VectorIndex, WebFetcher,
};
pub use std::sync::Arc;
pub use std::time::Duration;
pub use tokio::sync::{RwLock, mpsc};
