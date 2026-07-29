use crate::config::{
    Adapters, Governance, HarnessFeedbackAcks, JournalRecoveryClaims, RuntimeConfig,
};
use maestria_domain::{DomainError, DomainEventEnvelope, DomainInput, KernelState};
use std::sync::{Arc, atomic::AtomicU64};
use tokio::sync::{RwLock, mpsc, oneshot};

pub(crate) struct RuntimeCommand {
    pub(crate) correlation_id: u64,
    pub(crate) input: DomainInput,
    pub(crate) reply: oneshot::Sender<Result<DomainApplicationResult, RuntimeSubmissionError>>,
}

pub struct MaestriaRuntime {
    pub(crate) config: RuntimeConfig,
    pub(crate) state: Arc<RwLock<KernelState>>,
    pub(crate) adapters: Arc<Adapters>,
    pub(crate) governance: Arc<Governance>,
    pub(crate) input_tx: mpsc::Sender<DomainInput>,
    pub(crate) command_tx: mpsc::Sender<RuntimeCommand>,
    pub(crate) command_rx: Option<mpsc::Receiver<RuntimeCommand>>,
    pub(crate) next_command_id: Arc<AtomicU64>,
    pub(crate) journal_recovery_claims: JournalRecoveryClaims,
    pub(crate) next_validation_report_id: Arc<AtomicU64>,
    pub(crate) feedback_acks: HarnessFeedbackAcks,
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
    pub(crate) command_tx: mpsc::Sender<RuntimeCommand>,
    pub(crate) next_command_id: Arc<AtomicU64>,
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
