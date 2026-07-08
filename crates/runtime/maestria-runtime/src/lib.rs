use maestria_domain::{
    ApprovalDecision, DomainEvent, DomainEventEnvelope, DomainInput, EventId, HarnessRunCompleted,
    KernelState, MaestriaEffect, ParserResult, RegisterChunkInput, SequenceNumber,
    ValidationCompleted,
};
use maestria_governance::{
    ApprovalGate, ApprovalRequest, AutonomyProfile, ClassifyRisk, PolicyDecision, Scope, ScopeGuard,
};
use maestria_ports::{
    ArtifactRepository, BlobStore, EventLog, FileHandle, FileMetadata, FullTextIndex,
    HarnessAdapter, HarnessCommandClass, HarnessRequest, IndexedChunk, ParseContext, Parser,
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{mpsc, RwLock};

pub struct RuntimeConfig {
    pub profile: AutonomyProfile,
    pub input_buffer_size: usize,
    pub max_concurrent_effects: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            profile: AutonomyProfile::TrustedWorkspace,
            input_buffer_size: 1024,
            max_concurrent_effects: 16,
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
}

pub struct Governance {
    pub classifier: Arc<dyn ClassifyRisk + Send + Sync>,
    pub approval_gate: Arc<dyn ApprovalGate + Send + Sync>,
}

pub struct MaestriaRuntime {
    config: RuntimeConfig,
    state: Arc<RwLock<KernelState>>,
    adapters: Arc<Adapters>,
    governance: Arc<Governance>,
    input_tx: mpsc::Sender<DomainInput>,
}

pub struct RuntimeHandle {
    pub input_tx: mpsc::Sender<DomainInput>,
}

impl MaestriaRuntime {
    pub fn new(
        config: RuntimeConfig,
        state: KernelState,
        adapters: Adapters,
        governance: Governance,
    ) -> (Self, mpsc::Receiver<DomainInput>) {
        let (input_tx, input_rx) = mpsc::channel(config.input_buffer_size);
        (
            Self {
                config,
                state: Arc::new(RwLock::new(state)),
                adapters: Arc::new(adapters),
                governance: Arc::new(governance),
                input_tx,
            },
            input_rx,
        )
    }

    pub fn handle(&self) -> RuntimeHandle {
        RuntimeHandle {
            input_tx: self.input_tx.clone(),
        }
    }

    pub async fn run(self, mut input_rx: mpsc::Receiver<DomainInput>) {
        let (effect_tx, mut effect_rx) =
            mpsc::channel::<MaestriaEffect>(self.config.input_buffer_size);

        // Spawn effect executor
        let adapters = self.adapters.clone();
        let governance = self.governance.clone();
        let input_tx = self.input_tx.clone();
        let profile = self.config.profile;
        let state = self.state.clone();
        let max_concurrent_effects = self.config.max_concurrent_effects;

        tokio::spawn(async move {
            let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent_effects));
            while let Some(effect) = effect_rx.recv().await {
                let permit = semaphore.clone().acquire_owned().await.expect("semaphore closed");
                // Execute effect
                let adapters = adapters.clone();
                let input_tx = input_tx.clone();
                let governance = governance.clone();
                let profile = profile;
                let state = state.clone();

                tokio::spawn(async move {
                    Self::execute_effect(effect, adapters, governance, profile, state, input_tx)
                        .await;
                    drop(permit);
                });
            }
        });

        // Main domain loop
        while let Some(input) = input_rx.recv().await {
            let mut state = self.state.write().await;
            match state.apply_input(input) {
                Ok(output) => {
                    let _ = output.events;

                    // Dispatch effects
                    for effect in output.effects {
                        if let Err(e) = effect_tx.send(effect).await {
                            tracing::error!("Failed to dispatch effect: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Domain rejected input: {}", e);
                }
            }
        }
    }

    async fn execute_effect(
        effect: MaestriaEffect,
        adapters: Arc<Adapters>,
        governance: Arc<Governance>,
        profile: AutonomyProfile,
        state: Arc<RwLock<KernelState>>,
        input_tx: mpsc::Sender<DomainInput>,
    ) {
        let scope = ScopeGuard::new(Scope::default());
        let risk = governance.classifier.classify(&effect, &scope);
        let decision = governance.approval_gate.decide(&ApprovalRequest {
            effect: &effect,
            profile,
            scope: &scope,
        });

        match decision.decision {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => {
                tracing::warn!(?risk, %reason, "effect denied");
                return;
            }
            PolicyDecision::RequireApproval { reason } => {
                tracing::info!(?risk, %reason, "effect requires approval");
                return;
            }
        }

        match effect {
            MaestriaEffect::PersistEvent { event } => {
                let envelope = Self::envelope_for_event(&state, event.clone()).await;
                if let Err(error) = adapters.event_log.append(envelope) {
                    tracing::error!(%error, "failed to persist event");
                }
                if let DomainEvent::ArtifactRegistered { artifact_id, .. } = event {
                    let artifact = {
                        let state = state.read().await;
                        state.artifacts.get(&artifact_id).cloned()
                    };
                    if let Some(artifact) = artifact {
                        if let Err(error) = adapters.artifact_repo.put(artifact) {
                            tracing::error!(%artifact_id, %error, "failed to persist artifact metadata");
                        }
                    }
                }
            }
            MaestriaEffect::PersistState(request) => {
                let state = state.read().await;
                tracing::info!(
                    reason = %request.reason,
                    artifacts = state.artifacts.len(),
                    chunks = state.chunks.len(),
                    tasks = state.tasks.len(),
                    events = state.event_log.len(),
                    "state snapshot requested"
                );
            }
            MaestriaEffect::StoreBlob(request) => {
                match adapters.blob_store.put(request.payload.into_bytes()) {
                    Ok(blob_id) => tracing::debug!(
                        artifact_id = %request.artifact_id,
                        %blob_id,
                        "stored artifact blob"
                    ),
                    Err(error) => tracing::error!(
                        artifact_id = %request.artifact_id,
                        %error,
                        "failed to store artifact blob"
                    ),
                }
            }
            MaestriaEffect::ParseArtifact(request) => {
                let artifact = match adapters.artifact_repo.get(request.artifact_id) {
                    Ok(Some(artifact)) => artifact,
                    Ok(None) => {
                        let artifact = {
                            let state = state.read().await;
                            state.artifacts.get(&request.artifact_id).cloned()
                        };
                        let Some(artifact) = artifact else {
                            tracing::warn!(artifact_id = %request.artifact_id, "artifact missing for parse");
                            return;
                        };
                        artifact
                    }
                    Err(error) => {
                        tracing::error!(artifact_id = %request.artifact_id, %error, "failed to load artifact for parse");
                        return;
                    }
                };
                let path = PathBuf::from(&artifact.title);
                let bytes = artifact.title.clone().into_bytes();
                let metadata = FileMetadata {
                    path: path.clone(),
                    size: bytes.len(),
                    extension: path
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .map(str::to_owned),
                };
                if !adapters.parser.supports(&metadata) {
                    tracing::warn!(
                        artifact_id = %artifact.id,
                        parser = adapters.parser.id(),
                        path = %metadata.path.display(),
                        "parser does not support artifact"
                    );
                    return;
                }
                let file = FileHandle { path, bytes };
                match adapters.parser.parse(
                    file,
                    ParseContext {
                        artifact_id: artifact.id,
                    },
                ) {
                    Ok(parsed) => {
                        let chunks = parsed
                            .chunks
                            .into_iter()
                            .enumerate()
                            .map(|(order, chunk)| RegisterChunkInput {
                                chunk_id: chunk.chunk_id,
                                artifact_id: chunk.artifact_id,
                                order: (order.min(u32::MAX as usize)) as u32,
                                text: chunk.text,
                            })
                            .collect();
                        Self::send_input(
                            &input_tx,
                            DomainInput::ParserCompleted(ParserResult {
                                artifact_id: parsed.artifact_id,
                                chunks,
                                cards: parsed.cards,
                            }),
                            "parser completion",
                        )
                        .await;
                    }
                    Err(error) => {
                        tracing::error!(artifact_id = %artifact.id, %error, "parser failed");
                    }
                }
            }
            MaestriaEffect::IndexFullText(request) => {
                let chunk = {
                    let state = state.read().await;
                    state.chunks.get(&request.chunk_id).cloned()
                };
                let Some(chunk) = chunk else {
                    tracing::warn!(chunk_id = %request.chunk_id, "chunk missing for full-text index");
                    return;
                };
                if let Err(error) = adapters.search_index.index_chunks(vec![IndexedChunk {
                    artifact_id: request.artifact_id,
                    chunk_id: request.chunk_id,
                    text: chunk.text,
                }]) {
                    tracing::error!(chunk_id = %request.chunk_id, %error, "failed to index chunk");
                }
            }
            MaestriaEffect::IndexVector(request) => {
                tracing::debug!(
                    artifact_id = %request.artifact_id,
                    chunk_id = %request.chunk_id,
                    "vector index effect has no configured port"
                );
            }
            MaestriaEffect::UpdateGraph(request) => {
                tracing::debug!(
                    relation_id = %request.relation_id,
                    "graph update effect has no configured port"
                );
            }
            MaestriaEffect::QueryHarness(request) => {
                let class = match request.capability.as_str() {
                    "browser" => HarnessCommandClass::Browser,
                    "fetch" | "web" => HarnessCommandClass::Fetch,
                    _ => HarnessCommandClass::Shell,
                };
                let harness_request = HarnessRequest {
                    run_id: request.run_id,
                    command: request.command.clone(),
                    working_directory: match std::env::current_dir() {
                        Ok(p) => p,
                        Err(_) => PathBuf::from("."),
                    },
                    duration_budget: Duration::from_secs(60),
                    class,
                };
                match adapters.harness.execute(harness_request) {
                    Ok(outcome) => {
                        let mut output = String::from_utf8_lossy(&outcome.stdout).into_owned();
                        if !outcome.stderr.is_empty() {
                            if !output.is_empty() {
                                output.push('\n');
                            }
                            output.push_str(&String::from_utf8_lossy(&outcome.stderr));
                        }
                        Self::send_input(
                            &input_tx,
                            DomainInput::HarnessRunCompleted(HarnessRunCompleted {
                                task_id: request.task_id,
                                command: outcome.command,
                                exit_code: outcome.exit_code,
                                output,
                            }),
                            "harness completion",
                        )
                        .await;
                    }
                    Err(error) => tracing::error!(
                        run_id = %request.run_id,
                        %error,
                        "harness execution failed"
                    ),
                }
            }
            MaestriaEffect::FetchWeb(request) => {
                tracing::debug!(url = %request.url, "web fetch effect has no configured port");
            }
            MaestriaEffect::RunValidation(request) => {
                if let Some(claim_id) = request.claim_id {
                    Self::send_input(
                        &input_tx,
                        DomainInput::ValidationCompleted(ValidationCompleted {
                            claim_id,
                            valid: true,
                        }),
                        "validation completion",
                    )
                    .await;
                } else {
                    tracing::debug!(task_id = ?request.task_id, "validation effect has no claim to validate");
                }
            }
            MaestriaEffect::RequestApproval(request) => {
                tracing::info!(task_id = %request.task_id, "approval request requires external decision");
                Self::send_input(
                    &input_tx,
                    DomainInput::ApprovalResolved(ApprovalDecision {
                        task_id: request.task_id,
                        approved: false,
                    }),
                    "approval decision",
                )
                .await;
            }
            MaestriaEffect::EmitDiagnostic(diagnostic) => {
                tracing::info!(
                    task_id = ?diagnostic.task_id,
                    message = %diagnostic.message,
                    "domain diagnostic"
                );
            }
        }
    }

    async fn envelope_for_event(
        state: &Arc<RwLock<KernelState>>,
        event: DomainEvent,
    ) -> DomainEventEnvelope {
        let state = state.read().await;
        if let Some(envelope) = state
            .event_log
            .iter()
            .rev()
            .find(|envelope| envelope.event == event)
        {
            return envelope.clone();
        }

        let next = state.event_log.len() as u64 + 1;
        DomainEventEnvelope {
            id: EventId::new(next),
            sequence: SequenceNumber::new(next),
            event,
        }
    }

    async fn send_input(
        input_tx: &mpsc::Sender<DomainInput>,
        input: DomainInput,
        context: &'static str,
    ) {
        if let Err(error) = input_tx.send(input).await {
            tracing::error!(%error, context, "failed to send domain input");
        }
    }
}
