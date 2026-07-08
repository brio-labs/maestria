use maestria_domain::{
    ApprovalDecision, DomainEvent, DomainEventEnvelope, DomainInput, EventId, HarnessRunCompleted,
    KernelState, MaestriaEffect, ParserResult, RecordValidationReportInput, RegisterChunkInput,
    SequenceNumber, ValidationCompleted, ValidationReportId,
};
use maestria_governance::{
    ApprovalGate, ApprovalRequest, AutonomyProfile, ClassifyRisk, PolicyDecision, Scope, ScopeGuard,
};
use maestria_memory::MemoryService;
use maestria_ports::{
    ArtifactRepository, BlobStore, EventLog, FileHandle, FileMetadata, FullTextIndex,
    HarnessAdapter, HarnessCommandClass, HarnessRequest, IndexedChunk, ParseContext, Parser,
};
use maestria_validation::{ValidationContext, ValidationRunner};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc, RwLock};

pub struct RuntimeConfig {
    pub profile: AutonomyProfile,
    pub input_buffer_size: usize,
    pub max_concurrent_effects: usize,
    pub default_effect_timeout: Duration,
    pub max_retries: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            profile: AutonomyProfile::TrustedWorkspace,
            input_buffer_size: 1024,
            max_concurrent_effects: 16,
            default_effect_timeout: Duration::from_secs(300),
            max_retries: 3,
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
    next_validation_report_id: Arc<AtomicU64>,
}

pub struct RuntimeHandle {
    pub input_tx: mpsc::Sender<DomainInput>,
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
                next_validation_report_id,
            },
            input_rx,
        )
    }

    fn seed_next_validation_report_id(state: &KernelState) -> u64 {
        state
            .event_log
            .iter()
            .map(|entry| entry.id.value())
            .chain(state.validation_reports.keys().map(|id| id.value()))
            .max()
            .map_or(1, |value| value.saturating_add(1))
    }

    pub fn handle(&self) -> RuntimeHandle {
        RuntimeHandle {
            input_tx: self.input_tx.clone(),
        }
    }

    pub async fn run(
        self,
        mut input_rx: mpsc::Receiver<DomainInput>,
        shutdown_token: tokio_util::sync::CancellationToken,
    ) {
        let (effect_tx, mut effect_rx) =
            mpsc::channel::<MaestriaEffect>(self.config.input_buffer_size);

        // Spawn effect executor
        let adapters = self.adapters.clone();
        let governance = self.governance.clone();
        let input_tx = self.input_tx.clone();
        let profile = self.config.profile;
        let state = self.state.clone();
        let max_concurrent_effects = self.config.max_concurrent_effects;
        let default_effect_timeout = self.config.default_effect_timeout;
        let max_retries = self.config.max_retries;
        let effect_shutdown = shutdown_token.clone();
        let next_validation_report_id = self.next_validation_report_id.clone();

        tokio::spawn(async move {
            let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent_effects));
            loop {
                tokio::select! {
                    _ = effect_shutdown.cancelled() => {
                        tracing::info!("Effect executor shutting down");
                        break;
                    }
                    msg = effect_rx.recv() => {
                        let Some(mut effect) = msg else { break };
                        if let MaestriaEffect::RunValidation(request) = &mut effect {
                            request.validation_report_id = ValidationReportId::new(
                                next_validation_report_id.fetch_add(1, Ordering::Relaxed),
                            );
                        }
                        let permit = semaphore.clone().acquire_owned().await.expect("semaphore closed");
                        // Execute effect
                        let adapters = adapters.clone();
                        let input_tx = input_tx.clone();
                        let governance = governance.clone();
                        let state = state.clone();

                        tokio::spawn(async move {
                            let result = tokio::time::timeout(default_effect_timeout, async {
                                let mut attempts = 0;
                                loop {
                                    let success = Self::execute_effect(
                                        effect.clone(),
                                        adapters.clone(),
                                        governance.clone(),
                                        profile,
                                        state.clone(),
                                        input_tx.clone(),
                                    )
                                    .await;

                                    if success || attempts >= max_retries {
                                        break;
                                    }
                                    attempts += 1;
                                    tracing::warn!(
                                        "Retrying effect execution (attempt {})",
                                        attempts
                                    );
                                    tokio::time::sleep(Duration::from_millis(500 * (1 << attempts))).await;
                                }
                            })
                            .await;

                            if result.is_err() {
                                tracing::error!(
                                    "Watchdog: effect execution timed out after {:?}",
                                    default_effect_timeout
                                );
                            }
                            drop(permit);
                        });
                    }
                }
            }
        });
        // Main domain loop
        loop {
            tokio::select! {
                _ = shutdown_token.cancelled() => {
                    tracing::info!("Domain loop shutting down");
                    break;
                }
                msg = input_rx.recv() => {
                    let Some(input) = msg else { break };
                    let effects = {
                        let mut state = self.state.write().await;
                        match state.apply_input(input) {
                            Ok(output) => {
                                let _ = output.events;
                                output.effects
                            }
                            Err(e) => {
                                tracing::warn!("Domain rejected input: {}", e);
                                Vec::new()
                            }
                        }
                    };

                    // Dispatch effects (lock is dropped)
                    for effect in effects {
                        if let Err(e) = effect_tx.send(effect).await {
                            tracing::error!("Failed to dispatch effect: {}", e);
                        }
                    }
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
    ) -> bool {
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
                return true;
            }
            PolicyDecision::RequireApproval { reason } => {
                tracing::info!(?risk, %reason, "effect requires approval");
                return true;
            }
        }

        match effect {
            MaestriaEffect::PersistEvent { event } => {
                let envelope = Self::envelope_for_event(&state, event.clone()).await;
                if let Err(error) = adapters.event_log.append(envelope) {
                    tracing::error!(%error, "failed to persist event");
                    return false;
                }
                if let DomainEvent::ArtifactRegistered { artifact_id, .. } = event {
                    let artifact = {
                        let state = state.read().await;
                        state.artifacts.get(&artifact_id).cloned()
                    };
                    if let Some(artifact) = artifact {
                        if let Err(error) = adapters.artifact_repo.put(artifact) {
                            tracing::error!(%artifact_id, %error, "failed to persist artifact metadata");
                            return false;
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
                    Err(error) => {
                        tracing::error!(artifact_id = %request.artifact_id, %error, "failed to store artifact blob");
                        return false;
                    }
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
                            return true;
                        };
                        artifact
                    }
                    Err(error) => {
                        tracing::error!(artifact_id = %request.artifact_id, %error, "failed to load artifact for parse");
                        return true;
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
                    return true;
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
                        return false;
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
                    return true;
                };
                if let Err(error) = adapters.search_index.index_chunks(vec![IndexedChunk {
                    artifact_id: request.artifact_id,
                    chunk_id: request.chunk_id,
                    text: chunk.text,
                }]) {
                    tracing::error!(chunk_id = %request.chunk_id, %error, "failed to index chunk");
                    return false;
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
                    "shell" => HarnessCommandClass::Shell,
                    other => {
                        tracing::error!(capability = other, "Unknown harness capability requested");
                        return true;
                    }
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
                    Err(error) => {
                        tracing::error!(run_id = %request.run_id, %error, "harness execution failed");
                        return false;
                    }
                }
            }
            MaestriaEffect::FetchWeb(request) => {
                tracing::debug!(url = %request.url, "web fetch effect has no configured port");
            }
            MaestriaEffect::RunValidation(request) => {
                let report = {
                    let state = state.read().await;
                    let report_id = request.validation_report_id;
                    let task = request
                        .task_id
                        .and_then(|task_id| state.tasks.get(&task_id));
                    let harness_exit_code = request.task_id.and_then(|task_id| {
                        state
                            .event_log
                            .iter()
                            .rev()
                            .find_map(|entry| match entry.event {
                                DomainEvent::HarnessRunCompleted {
                                    task_id: Some(event_task_id),
                                    exit_code,
                                    ..
                                } if event_task_id == task_id => Some(exit_code),
                                _ => None,
                            })
                    });
                    let mut claims = BTreeMap::new();
                    if let Some(claim_id) = request.claim_id {
                        if let Some(claim) = state.claims.get(&claim_id) {
                            claims.insert(claim_id, claim.clone());
                        } else {
                            tracing::warn!(claim_id = ?claim_id, "validation requested for missing claim");
                        }
                    } else {
                        claims.extend(state.claims.iter().map(|(id, claim)| (*id, claim.clone())));
                    }
                    let evidences = state
                        .evidences
                        .iter()
                        .map(|(id, evidence)| (*id, evidence.clone()))
                        .collect();
                    let memory_candidates = state
                        .memory_candidates
                        .iter()
                        .map(|(id, candidate)| (*id, candidate.clone()))
                        .collect();
                    let review_queue =
                        MemoryService::review_queue(&state.memory_candidates, &state.memories);
                    if !review_queue.is_empty() {
                        tracing::debug!(
                            pending_candidates = review_queue.len(),
                            "validation found queued memory candidates"
                        );
                    }

                    let mut validators: Vec<Box<dyn maestria_validation::Validator>> = vec![
                        Box::new(maestria_validation::CitationValidator),
                        Box::new(maestria_validation::EvidenceExistenceValidator),
                        Box::new(maestria_validation::MemoryValidator),
                        Box::new(maestria_validation::HarnessRunValidator),
                    ];
                    if request.task_id.is_some() {
                        validators.push(Box::new(maestria_validation::TaskStateValidator));
                    }

                    let runner = ValidationRunner::with_validators(validators);
                    runner.run(
                        report_id,
                        request.task_id,
                        &ValidationContext {
                            task,
                            claims: &claims,
                            evidences: &evidences,
                            memory_candidates: &memory_candidates,
                            harness_exit_code,
                        },
                    )
                };
                if let Some(claim_id) = request.claim_id {
                    Self::send_input(
                        &input_tx,
                        DomainInput::ValidationCompleted(ValidationCompleted {
                            claim_id,
                            valid: report.passed,
                        }),
                        "validation completion",
                    )
                    .await;
                } else {
                    tracing::debug!(task_id = ?request.task_id, "validation effect has no claim to validate");
                }
                Self::send_input(
                    &input_tx,
                    DomainInput::RecordValidationReport(RecordValidationReportInput {
                        report_id: report.id,
                        task_id: request.task_id,
                        passed: report.passed,
                        warnings: report.warnings,
                    }),
                    "validation report",
                )
                .await;
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
        true
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
        let next = state
            .event_log
            .iter()
            .flat_map(|envelope| {
                std::iter::once(envelope.id.value())
                    .chain(std::iter::once(envelope.sequence.value()))
            })
            .max()
            .map_or(0, |value| value.saturating_add(1));
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
