use crate::config::{
    Adapters, DegradedVectorArtifacts, FullTextLocks, Governance, HarnessFeedbackAcks,
    JournalRecoveryClaims, RuntimeConfig,
};
use maestria_domain::{DomainError, DomainEventEnvelope, DomainInput, HarnessRunId, KernelState};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, atomic::AtomicU64};
use tokio::sync::{RwLock, mpsc, oneshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectPreparation {
    Deferred,
    BeforeReply,
}

pub(crate) struct RuntimeCommand {
    pub(crate) correlation_id: u64,
    pub(crate) effect_preparation: EffectPreparation,
    pub(crate) reply: oneshot::Sender<Result<DomainApplicationResult, RuntimeSubmissionError>>,
}

pub(crate) struct PendingApplication {
    pub(crate) command: RuntimeCommand,
    pub(crate) outcome: DomainApplicationResult,
}

pub struct MaestriaRuntime {
    pub(crate) config: RuntimeConfig,
    pub(crate) state: Arc<RwLock<KernelState>>,
    pub(crate) adapters: Arc<Adapters>,
    pub(crate) governance: Arc<Governance>,
    pub(crate) input_tx: mpsc::Sender<DomainInput>,
    pub(crate) command_tx: mpsc::Sender<(DomainInput, RuntimeCommand)>,
    pub(crate) command_rx: Option<mpsc::Receiver<(DomainInput, RuntimeCommand)>>,
    pub(crate) next_command_id: Arc<AtomicU64>,
    pub(crate) journal_recovery_claims: JournalRecoveryClaims,
    pub(crate) feedback_acks: HarnessFeedbackAcks,
    pub(crate) degraded_vector_artifacts: DegradedVectorArtifacts,
    pub(crate) full_text_locks: FullTextLocks,
    pub(crate) pending_applications: Mutex<BTreeMap<HarnessRunId, PendingApplication>>,
    pub(crate) pending_notebook_drafts: Mutex<BTreeMap<u64, RuntimeCommand>>,
    #[cfg(test)]
    pub(crate) test_pre_failed_effect_task: bool,
}
/// Correlated result returned after the domain accepted an input and the
/// complete emitted effect batch crossed runtime admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainApplicationResult {
    pub correlation_id: u64,
    pub events: Vec<DomainEventEnvelope>,
    pub effects_admitted: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeRunError {
    RecoveryPlanning { reason: String },
    CommandReceiverUnavailable,
    EffectExecutorJoin { reason: String },
}

impl std::fmt::Display for RuntimeRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RecoveryPlanning { reason } => {
                write!(formatter, "model-agent recovery planning failed: {reason}")
            }
            Self::CommandReceiverUnavailable => {
                formatter.write_str("runtime command receiver is unavailable")
            }
            Self::EffectExecutorJoin { reason } => {
                write!(formatter, "effect executor task failed: {reason}")
            }
        }
    }
}

impl std::error::Error for RuntimeRunError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSubmissionError {
    CapacityFull,
    RuntimeShutdown,
    DomainRejected {
        correlation_id: u64,
        error: DomainError,
    },
    EffectAdmissionRejected {
        correlation_id: u64,
    },
    EffectPreparationRejected {
        correlation_id: u64,
        reason: String,
    },
    PersistenceBarrierFailed {
        correlation_id: u64,
    },
}

impl std::fmt::Display for RuntimeSubmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CapacityFull => f.write_str("runtime command capacity is full"),
            Self::RuntimeShutdown => f.write_str("runtime command channel is closed"),
            Self::DomainRejected { error, .. } => write!(f, "domain rejected input: {error}"),
            Self::EffectAdmissionRejected { .. } => {
                f.write_str("runtime rejected the emitted effect batch")
            }
            Self::EffectPreparationRejected { reason, .. } => {
                write!(f, "runtime rejected effect before reply: {reason}")
            }
            Self::PersistenceBarrierFailed { .. } => {
                f.write_str("runtime could not confirm durable proposal persistence")
            }
        }
    }
}

impl std::error::Error for RuntimeSubmissionError {}

#[derive(Clone)]
pub struct RuntimeHandle {
    pub(crate) input_tx: mpsc::Sender<DomainInput>,
    pub(crate) command_tx: mpsc::Sender<(DomainInput, RuntimeCommand)>,
    pub(crate) next_command_id: Arc<AtomicU64>,
    pub(crate) id_allocator: Arc<dyn maestria_ports::IdAllocator + Send + Sync>,
    pub(crate) search_executor:
        Option<Arc<dyn maestria_ports::SearchKnowledgeExecutor + Send + Sync>>,
    pub(crate) realm_read_grant_repo:
        Arc<dyn maestria_ports::RealmReadGrantRepository + Send + Sync>,
    pub(crate) state: Arc<RwLock<KernelState>>,
}

impl RuntimeHandle {
    /// The runtime-owned knowledge-search executor, when one was configured.
    ///
    /// Application entry points that run a governed search reuse this executor
    /// instead of assembling a second search runtime beside the live one
    /// (R28: lifecycle orchestration has one owner).
    pub fn search_executor(
        &self,
    ) -> Option<Arc<dyn maestria_ports::SearchKnowledgeExecutor + Send + Sync>> {
        self.search_executor.clone()
    }

    pub fn realm_read_grant_repository(
        &self,
    ) -> Arc<dyn maestria_ports::RealmReadGrantRepository + Send + Sync> {
        self.realm_read_grant_repo.clone()
    }

    /// Snapshot the current replayed domain state for a request-bound,
    /// read-only provider runtime.
    ///
    /// # Cancellation
    /// Cancelling while waiting for the read lock leaves runtime state unchanged.
    pub async fn kernel_state(&self) -> KernelState {
        self.state.read().await.clone()
    }

    /// Read a single artifact without cloning the whole kernel state.
    ///
    /// Used by index ingestion to avoid per-file full-state clones.
    ///
    /// # Cancellation
    /// Cancelling while waiting for the read lock leaves runtime state unchanged.
    pub async fn artifact(
        &self,
        id: maestria_domain::ArtifactId,
    ) -> Option<maestria_domain::Artifact> {
        self.state.read().await.artifacts.get(&id).cloned()
    }
}

/// Reserved capacity for one correlated runtime submission.
///
/// Reserving is cancellation-safe: the caller retains its input until this value is returned.
/// [`RuntimeSubmissionPermit::submit`] accepts the input synchronously on its first poll before
/// awaiting the correlated result.
pub struct RuntimeSubmissionPermit {
    pub(crate) permit: mpsc::OwnedPermit<(DomainInput, RuntimeCommand)>,
    pub(crate) correlation_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackError {
    CapacityFull,
    RuntimeShutdown,
}
impl std::fmt::Display for FeedbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedbackError::CapacityFull => write!(f, "capacity full"),
            FeedbackError::RuntimeShutdown => write!(f, "runtime shutdown"),
        }
    }
}

impl std::error::Error for FeedbackError {}
