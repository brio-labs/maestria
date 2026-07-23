mod approval;
mod completion;
/// Responsibility map:
/// - `config`: module responsibility.
/// - `effect_dispatch`: module responsibility.
/// - `effect_execution`: module responsibility.
/// - `effect_result`: module responsibility.
/// - `harness`: module responsibility.
/// - `indexing`: module responsibility.
/// - `parser_mapping`: module responsibility.
/// - `parsing`: module responsibility.
/// - `parsing_records`: module responsibility.
/// - `persistence`: module responsibility.
/// - `shell_policy`: module responsibility.
/// - `supervision`: module responsibility.
/// - `validation`: module responsibility.
/// - `vector_indexing`: module responsibility.
/// - `web_evidence`: module responsibility.
/// - `approval`: module responsibility.
/// - `completion`: module responsibility.
mod config;
mod effect_dispatch;
mod effect_execution;
mod effect_result;
mod harness;
mod indexing;
mod parser_mapping;
mod parsing;
mod parsing_records;
mod persistence;
mod shell_policy;
mod supervision;
mod validation;
mod vector_indexing;
mod web_evidence;

#[cfg(test)]
pub use config::EffectExecutionContext;
use config::EffectExecutionContext as ExecutionContext;
use config::HarnessFeedbackAcks;
pub use config::{Adapters, Governance, RuntimeConfig};
use maestria_domain::{DomainInput, KernelState, MaestriaEffect, ValidationReportId};
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{RwLock, mpsc};
pub struct MaestriaRuntime {
    config: RuntimeConfig,
    state: Arc<RwLock<KernelState>>,
    adapters: Arc<Adapters>,
    governance: Arc<Governance>,
    input_tx: mpsc::Sender<DomainInput>,
    next_validation_report_id: Arc<AtomicU64>,
    feedback_acks: HarnessFeedbackAcks,
}

pub struct RuntimeHandle {
    pub input_tx: mpsc::Sender<DomainInput>,
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

impl RuntimeHandle {
    pub fn try_send_feedback(&self, input: DomainInput) -> Result<(), FeedbackError> {
        self.input_tx.try_send(input).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => FeedbackError::CapacityFull,
            mpsc::error::TrySendError::Closed(_) => FeedbackError::RuntimeShutdown,
        })
    }
}

impl MaestriaRuntime {
    pub fn new(
        mut config: RuntimeConfig,
        state: KernelState,
        adapters: Adapters,
        governance: Governance,
    ) -> (Self, mpsc::Receiver<DomainInput>) {
        config.max_concurrent_effects = config.max_concurrent_effects.max(1);
        let (input_tx, input_rx) = mpsc::channel(config.input_buffer_size);
        let next_validation_report_id =
            Arc::new(AtomicU64::new(Self::seed_next_validation_report_id(&state)));
        (
            Self {
                config,
                state: Arc::new(RwLock::new(state)),
                adapters: Arc::new(adapters),
                governance: Arc::new(governance),
                input_tx,
                feedback_acks: Arc::new(Mutex::new(BTreeMap::new())),
                next_validation_report_id,
            },
            input_rx,
        )
    }

    fn seed_next_validation_report_id(state: &KernelState) -> u64 {
        state
            .validation_reports
            .keys()
            .map(|id| id.value())
            .max()
            .map_or(1, |value| value.saturating_add(1))
    }

    pub fn handle(&self) -> RuntimeHandle {
        RuntimeHandle {
            input_tx: self.input_tx.clone(),
        }
    }

    pub fn with_graceful_shutdown(mut self) -> Self {
        self.config.drain_effects_on_shutdown = true;
        self
    }
    pub async fn snapshot_state(&self) -> KernelState {
        self.state.read().await.clone()
    }

    /// Allocate a claim ID and a memory-candidate ID through the
    /// runtime's configured `IdAllocator`.
    pub fn allocate_memory_proposal_ids(
        &self,
    ) -> Result<
        (maestria_domain::ClaimId, maestria_domain::MemoryCandidateId),
        maestria_ports::PortError,
    > {
        let claim_id = self.adapters.id_allocator.allocate_claim_id()?;
        let candidate_id = self.adapters.id_allocator.allocate_memory_candidate_id()?;
        Ok((claim_id, candidate_id))
    }

    /// Runs the domain-input loop until the shutdown token is cancelled or
    /// the input channel closes.
    ///
    /// Cancellation stops accepting new inputs. By default, in-flight effects
    /// are cancelled; call [`Self::with_graceful_shutdown`] before `run` to
    /// drain already-started effects. The method returns after the effect
    /// executor has observed the selected shutdown policy.
    pub async fn run(
        self,
        mut input_rx: mpsc::Receiver<DomainInput>,
        shutdown_token: tokio_util::sync::CancellationToken,
    ) {
        let (effect_tx, effect_rx) = mpsc::channel::<MaestriaEffect>(self.config.input_buffer_size);
        let effect_shutdown = tokio_util::sync::CancellationToken::new();
        let effect_executor =
            self.spawn_effect_executor(effect_rx, effect_shutdown.clone(), shutdown_token.clone());

        loop {
            tokio::select! {
                () = shutdown_token.cancelled() => break,
                msg = input_rx.recv() => {
                    let Some(input) = msg else { break };

                    match &input {
                        DomainInput::ApprovalResolved(decision)
                            if !self.check_approval_boundary(decision) =>
                        {
                            continue;
                        }
                        DomainInput::CompleteTask(complete_input) => {
                            let valid = self.check_completion_validation(complete_input).await;
                            if !valid {
                                continue;
                            }
                        }
                        DomainInput::HarnessRunCompleted(completion)
                            if !self.check_harness_feedback_boundary(completion) =>
                        {
                            continue;
                        }
                        _ => {}
                    }

                    let harness_feedback = match &input {
                        DomainInput::HarnessRunCompleted(completion) => {
                            Some((completion.run_id, completion.generation))
                        }
                        _ => None,
                    };
                    let mut wait_for_report_id = None;
                    let effects = {
                        let mut state = self.state.write().await;
                        match state.apply_input(input) {
                            Ok(output) => {
                                for effect in &output.effects {
                                    if let maestria_domain::MaestriaEffect::PersistEvent { envelope } = effect
                                        && let maestria_domain::DomainEvent::ValidationReportCreated { report_id, .. } = &envelope.event
                                    {
                                        wait_for_report_id = Some(*report_id);
                                    }
                                }
                                self.register_harness_feedback(harness_feedback, &output.effects);
                                output.effects
                            }
                            Err(error) => {
                                tracing::warn!(%error, "domain rejected input");
                                Vec::new()
                            }
                        }
                    };
                    if !self
                        .dispatch_effects(effects, &effect_tx, &shutdown_token)
                        .await
                    {
                        break;
                    }
                    if let Some(report_id) = wait_for_report_id {

                        let found = self
                            .wait_for_validation_report(report_id, &shutdown_token)
                            .await;
                        if !found {
                            if !shutdown_token.is_cancelled() {
                                tracing::error!(
                                    "fatal: timeout or error waiting for durable ValidationReportCreated; stopping runtime"
                                );
                                shutdown_token.cancel();
                            }
                            break;
                        }
                    }
                }
            }
        }

        drop(effect_tx);
        if !self.config.drain_effects_on_shutdown {
            effect_shutdown.cancel();
        }
        shutdown_token.cancel();
        let _ = effect_executor.await;
    }

    async fn wait_for_validation_report(
        &self,
        report_id: maestria_domain::ValidationReportId,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        let check = async {
            loop {
                if shutdown_token.is_cancelled() {
                    return false;
                }
                match self
                    .adapters
                    .event_log
                    .scan(maestria_ports::EventFilter { artifact_id: None })
                {
                    Ok(events) => {
                        if events.iter().any(|env| {
                            matches!(
                                &env.event,
                                maestria_domain::DomainEvent::ValidationReportCreated {
                                    report_id: id,
                                    ..
                                } if *id == report_id
                            )
                        }) {
                            return true;
                        }
                    }
                    Err(error) => {
                        tracing::error!(
                            %error,
                            "failed to scan event log during validation report barrier"
                        );
                        return false;
                    }
                }
                tokio::select! {
                    () = shutdown_token.cancelled() => return false,
                    () = tokio::time::sleep(std::time::Duration::from_millis(5)) => {}
                }
            }
        };
        matches!(
            tokio::time::timeout(self.config.default_effect_timeout, check).await,
            Ok(true)
        )
    }

    fn spawn_effect_executor(
        &self,
        mut receiver: mpsc::Receiver<MaestriaEffect>,
        effect_shutdown: tokio_util::sync::CancellationToken,
        runtime_shutdown: tokio_util::sync::CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let adapters = Arc::clone(&self.adapters);
        let governance = Arc::clone(&self.governance);
        let input_tx = self.input_tx.clone();
        let state = Arc::clone(&self.state);
        let profile = self.config.profile;
        let scope = self.config.scope.clone();
        let max_concurrent_effects = self.config.max_concurrent_effects;
        let default_effect_timeout = self.config.default_effect_timeout;
        let max_retries = self.config.max_retries;
        let scope_id = self.config.scope_id;
        let next_validation_report_id = Arc::clone(&self.next_validation_report_id);
        let feedback_acks = Arc::clone(&self.feedback_acks);
        let embedding_model = self.config.embedding_model.clone();
        let drain_effects_on_shutdown = self.config.drain_effects_on_shutdown;
        tokio::spawn(async move {
            let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent_effects));
            let mut in_flight = tokio::task::JoinSet::new();
            loop {
                while in_flight.try_join_next().is_some() {}
                tokio::select! {
                    biased;
                    () = effect_shutdown.cancelled() => break,
                    message = receiver.recv() => {
                        let Some(mut effect) = message else { break };
                        if effect_shutdown.is_cancelled() {
                            break;
                        }
                        if let MaestriaEffect::RunValidation(request) = &mut effect {
                            request.validation_report_id = ValidationReportId::new(
                                next_validation_report_id.fetch_add(1, Ordering::Relaxed),
                            );
                        }
                        let context = ExecutionContext {
                            adapters: Arc::clone(&adapters),
                            governance: Arc::clone(&governance),
                            profile,
                            scope: scope.clone(),
                            scope_id,
                            state: Arc::clone(&state),
                            input_tx: input_tx.clone(),
                            embedding_model: embedding_model.clone(),
                            feedback_acks: Arc::clone(&feedback_acks),
                            default_effect_timeout,
                            max_retries,
                        };
                        if matches!(&effect, MaestriaEffect::PersistEvent { .. }) {
                            if context.execute_with_retries(effect).await.is_err() {
                                effect_shutdown.cancel();
                                runtime_shutdown.cancel();
                                break;
                            }
                            continue;
                        }
                        let permit = tokio::select! {
                            biased;
                            () = effect_shutdown.cancelled() => break,
                            permit = Arc::clone(&semaphore).acquire_owned() => {
                                match permit {
                                    Ok(permit) => permit,
                                    Err(_) => break,
                                }
                            }
                        };
                        let shutdown = effect_shutdown.clone();
                        let runtime_shutdown = runtime_shutdown.clone();
                        in_flight.spawn(async move {
                            if let Err(error) = context.execute_with_retries(effect).await {
                                tracing::error!(%error, "spawned effect failed");
                                if error.fatal() {
                                    tracing::error!("fatal spawned effect failure; cancelling runtime execution");
                                    shutdown.cancel();
                                    runtime_shutdown.cancel();
                                }
                            }
                            drop(permit);
                        });
                    }
                }
            }
            if drain_effects_on_shutdown && !effect_shutdown.is_cancelled() {
                while in_flight.join_next().await.is_some() {}
            } else {
                in_flight.shutdown().await;
            }
        })
    }
}

#[cfg(test)]
impl MaestriaRuntime {
    pub(crate) async fn test_execute_effect(
        effect: MaestriaEffect,
        context: ExecutionContext,
        persistence_barrier_timeout: Option<std::time::Duration>,
    ) -> bool {
        context
            .execute_effect(effect, persistence_barrier_timeout)
            .await
            .is_ok()
    }
}

#[cfg(test)]
pub mod test_support {
    pub use super::{
        Adapters, EffectExecutionContext, FeedbackError, Governance, MaestriaRuntime, RuntimeConfig,
    };
    pub use maestria_domain::{
        DomainEvent, DomainEventEnvelope, DomainInput, KernelState, MaestriaEffect,
        ValidationReportId, content_hash, evidence_id_for,
    };
    pub use maestria_governance::{AutonomyProfile, Scope};
    pub use maestria_ports::{
        ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EffectJournal,
        EffectJournalEntry, EffectJournalIntent, EffectJournalStatus, EventFilter, EventLog,
        EvidenceRepository, FullTextIndex, GraphIndex, HarnessAdapter, HarnessCommandClass,
        HarnessRequest, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
        InMemoryChunkRepository, InMemoryEffectJournal, InMemoryEventLog,
        InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryGraphIndex,
        InMemoryHarnessAdapter, InMemoryParser, InMemoryVectorIndex, InMemoryWebFetcher,
        IndexedCard, IndexedChunk, ParseContext, Parser, PortError, SourceSpan, VectorEmbedding,
        VectorIndex, WebFetcher,
    };
    pub use std::sync::Arc;
    pub use std::time::Duration;
    pub use tokio::sync::{RwLock, mpsc};
}

#[cfg(test)]
mod test_helpers;
#[cfg(test)]
mod tests;
