use maestria_domain::{
    ApprovalDecision, DomainEvent, DomainInput, EvidenceKind, FullTextIndexCompleted,
    HarnessRunCompleted, KernelState, LogicalTick, MaestriaEffect, ParserResult,
    RecordEvidenceInput, RecordValidationReportInput, RegisterChunkInput, ValidationCompleted,
    ValidationReportId, content_hash, evidence_id_for, excerpt_for, line_range_for_chunk,
};
use maestria_governance::{
    ApprovalGate, ApprovalRequest, AutonomyProfile, ClassifyRisk, PolicyDecision, Scope, ScopeGuard,
};
use maestria_memory::MemoryService;
use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EventFilter, EventLog,
    EvidenceRepository, FileHandle, FileMetadata, FullTextIndex, GraphIndex, HarnessAdapter,
    HarnessCommandClass, HarnessRequest, IndexedChunk, ParseContext, Parser, PortError,
    VectorEmbedding, VectorIndex, WebFetcher,
};
use maestria_validation::{ValidationContext, ValidationRunner};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{RwLock, mpsc};

pub struct RuntimeConfig {
    pub profile: AutonomyProfile,
    pub scope: Scope,
    pub input_buffer_size: usize,
    pub max_concurrent_effects: usize,
    pub default_effect_timeout: Duration,
    pub max_retries: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            profile: AutonomyProfile::TrustedWorkspace,
            scope: Scope::default(),
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
    pub chunk_repo: Arc<dyn ChunkRepository + Send + Sync>,
    pub card_repo: Arc<dyn CardRepository + Send + Sync>,
    pub evidence_repo: Arc<dyn EvidenceRepository + Send + Sync>,
    pub vector_index: Arc<dyn VectorIndex + Send + Sync>,
    pub graph_index: Arc<dyn GraphIndex + Send + Sync>,
    pub web_fetcher: Arc<dyn WebFetcher + Send + Sync>,
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

#[derive(Clone)]
struct EffectExecutionContext {
    adapters: Arc<Adapters>,
    governance: Arc<Governance>,
    profile: AutonomyProfile,
    scope: Scope,
    state: Arc<RwLock<KernelState>>,
    input_tx: mpsc::Sender<DomainInput>,
    default_effect_timeout: Duration,
    max_retries: u32,
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
    pub async fn snapshot_state(&self) -> KernelState {
        self.state.read().await.clone()
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
        let scope = self.config.scope.clone();
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
                        let Ok(permit) = semaphore.clone().acquire_owned().await else {
                            tracing::warn!("Effect executor semaphore closed");
                            break;
                        };
                        let persist_event =
                            matches!(&effect, MaestriaEffect::PersistEvent { .. });
                        let context = EffectExecutionContext {
                            adapters: adapters.clone(),
                            governance: governance.clone(),
                            profile,
                            scope: scope.clone(),
                            state: state.clone(),
                            input_tx: input_tx.clone(),
                            default_effect_timeout,
                            max_retries,
                        };

                        if persist_event {
                            let success = Self::execute_with_retries(effect, context).await;
                            drop(permit);
                            if !success {
                                tracing::error!(
                                    "fatal event persistence failure; stopping runtime"
                                );
                                effect_shutdown.cancel();
                                break;
                            }
                            continue;
                        }

                        tokio::spawn(async move {
                            Self::execute_with_retries(effect, context).await;
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
                        tokio::select! {
                            _ = shutdown_token.cancelled() => break,
                            result = effect_tx.send(effect) => {
                                if let Err(error) = result {
                                    tracing::error!("Failed to dispatch effect: {}", error);
                                    shutdown_token.cancel();
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn execute_with_retries(effect: MaestriaEffect, context: EffectExecutionContext) -> bool {
        let result = tokio::time::timeout(context.default_effect_timeout, async {
            let mut attempts = 0;
            loop {
                let success = Self::execute_effect(
                    effect.clone(),
                    context.adapters.clone(),
                    context.governance.clone(),
                    context.profile,
                    context.scope.clone(),
                    context.state.clone(),
                    context.input_tx.clone(),
                )
                .await;

                if success || attempts >= context.max_retries {
                    return success;
                }
                attempts += 1;
                tracing::warn!("Retrying effect execution (attempt {})", attempts);
                tokio::time::sleep(Duration::from_millis(500 * (1 << attempts))).await;
            }
        })
        .await;

        match result {
            Ok(success) => success,
            Err(_) => {
                tracing::error!(
                    "Watchdog: effect execution timed out after {:?}",
                    context.default_effect_timeout
                );
                false
            }
        }
    }

    async fn execute_effect(
        effect: MaestriaEffect,
        adapters: Arc<Adapters>,
        governance: Arc<Governance>,
        profile: AutonomyProfile,
        configured_scope: Scope,
        state: Arc<RwLock<KernelState>>,
        input_tx: mpsc::Sender<DomainInput>,
    ) -> bool {
        let scope = ScopeGuard::new(configured_scope);
        let risk = governance.classifier.classify(&effect, &scope);
        let decision = governance.approval_gate.decide(&ApprovalRequest {
            effect: &effect,
            profile,
            scope: &scope,
        });

        let persistence_effect = matches!(&effect, MaestriaEffect::PersistEvent { .. });
        match decision.decision {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => {
                tracing::warn!(?risk, %reason, "effect denied");
                return !persistence_effect;
            }
            PolicyDecision::RequireApproval { reason } => {
                tracing::info!(?risk, %reason, "effect requires approval");
                return !persistence_effect;
            }
        }

        match effect {
            MaestriaEffect::PersistEvent { envelope } => {
                let persisted = match adapters.event_log.append(envelope.clone()) {
                    Ok(()) => true,
                    Err(PortError::Conflict { .. }) => {
                        match adapters.event_log.scan(EventFilter { artifact_id: None }) {
                            Ok(events) if events.iter().any(|stored| stored == &envelope) => true,
                            Ok(_) => {
                                tracing::error!(
                                    "event persistence conflict for a different envelope"
                                );
                                false
                            }
                            Err(error) => {
                                tracing::error!(%error, "failed to verify persisted event after conflict");
                                false
                            }
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "failed to persist event");
                        false
                    }
                };
                if !persisted {
                    return false;
                }

                match &envelope.event {
                    DomainEvent::ArtifactRegistered { artifact_id, .. } => {
                        let artifact = {
                            let state = state.read().await;
                            state.artifacts.get(artifact_id).cloned()
                        };
                        if let Some(artifact) = artifact {
                            if let Err(error) = adapters.artifact_repo.put(artifact) {
                                tracing::error!(%artifact_id, %error, "failed to persist artifact");
                                return false;
                            }
                        } else {
                            tracing::error!(%artifact_id, "artifact missing from state during persist");
                            return false;
                        }
                    }
                    DomainEvent::ChunkRegistered { chunk_id, .. } => {
                        let chunk = {
                            let state = state.read().await;
                            state.chunks.get(chunk_id).cloned()
                        };
                        if let Some(chunk) = chunk {
                            if let Err(error) = adapters.chunk_repo.put(chunk) {
                                tracing::error!(%chunk_id, %error, "failed to persist chunk");
                                return false;
                            }
                        } else {
                            tracing::error!(%chunk_id, "chunk missing from state during persist");
                            return false;
                        }
                    }
                    DomainEvent::CardCreated { card_id, .. } => {
                        let card = {
                            let state = state.read().await;
                            state.cards.get(card_id).cloned()
                        };
                        if let Some(card) = card {
                            if let Err(error) = adapters.card_repo.put(card) {
                                tracing::error!(%card_id, %error, "failed to persist card");
                                return false;
                            }
                        } else {
                            tracing::error!(%card_id, "card missing from state during persist");
                            return false;
                        }
                    }
                    DomainEvent::EvidenceRecorded { evidence_id, .. } => {
                        let evidence = {
                            let state = state.read().await;
                            state.evidences.get(evidence_id).cloned()
                        };
                        if let Some(evidence) = evidence {
                            if let Err(error) = adapters.evidence_repo.put(evidence) {
                                tracing::error!(%evidence_id, %error, "failed to persist evidence");
                                return false;
                            }
                        } else {
                            tracing::error!(%evidence_id, "evidence missing from state during persist");
                            return false;
                        }
                    }
                    DomainEvent::PendingIndex { artifact_id, .. }
                    | DomainEvent::ArtifactIndexed { artifact_id } => {
                        let artifact = {
                            let state = state.read().await;
                            state.artifacts.get(artifact_id).cloned()
                        };
                        if let Some(artifact) = artifact {
                            if let Err(error) = adapters.artifact_repo.put(artifact) {
                                tracing::error!(%artifact_id, %error, "failed to persist artifact update");
                                return false;
                            }
                        } else {
                            tracing::error!(%artifact_id, "artifact missing from state during index-status persist");
                            return false;
                        }
                    }
                    _ => {}
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
            MaestriaEffect::StoreBlob(request) => match adapters.blob_store.put(request.payload) {
                Ok(blob_id) => tracing::debug!(
                    artifact_id = %request.artifact_id,
                    %blob_id,
                    "stored artifact blob"
                ),
                Err(error) => {
                    tracing::error!(artifact_id = %request.artifact_id, %error, "failed to store artifact blob");
                    return false;
                }
            },
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
                let path = PathBuf::from(&request.source_path);
                let bytes = request.source_bytes.clone();
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
                        // Store source bytes to obtain the immutable BlobId for evidence snapshots.
                        // NOTE: this duplicates the separate StoreBlob effect produced by the domain;
                        // idempotent blob storage is acceptable for this bounded slice.
                        let blob_id = match adapters.blob_store.put(request.source_bytes.clone()) {
                            Ok(id) => id,
                            Err(error) => {
                                tracing::error!(artifact_id = %artifact.id, %error, "failed to store source blob for evidence");
                                return false;
                            }
                        };
                        let source_hash = content_hash(&request.source_bytes);
                        let source_text = String::from_utf8_lossy(&request.source_bytes);
                        let observed_at = LogicalTick::new(1);
                        let mut search_start: usize = 0;

                        for (order, chunk) in parsed.chunks.iter().enumerate() {
                            let range =
                                line_range_for_chunk(&source_text, &chunk.text, &mut search_start);
                            let evidence_id = evidence_id_for(artifact.id, order as u32);
                            let excerpt = excerpt_for(&chunk.text);
                            Self::send_input(
                                &input_tx,
                                DomainInput::RecordEvidence(RecordEvidenceInput {
                                    evidence_id,
                                    artifact_id: artifact.id,
                                    claim_id: None,
                                    kind: EvidenceKind::FileSpan {
                                        path: request.source_path.clone(),
                                        range,
                                        content_hash: source_hash.clone(),
                                        snapshot: Some(blob_id),
                                    },
                                    excerpt,
                                    observed_at,
                                }),
                                "record evidence",
                            )
                            .await;
                        }

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
                Self::send_input(
                    &input_tx,
                    DomainInput::FullTextIndexCompleted(FullTextIndexCompleted {
                        artifact_id: request.artifact_id,
                        chunk_id: request.chunk_id,
                    }),
                    "full-text index completion",
                )
                .await;
            }
            MaestriaEffect::IndexVector(request) => {
                let chunk = {
                    let state = state.read().await;
                    state.chunks.get(&request.chunk_id).cloned()
                };
                let Some(chunk) = chunk else {
                    tracing::warn!(chunk_id = %request.chunk_id, "chunk missing for vector index");
                    return true;
                };
                let embedding = VectorEmbedding {
                    chunk_id: request.chunk_id,
                    vector: Vec::new(),
                    provenance: maestria_ports::EmbeddingProvenance {
                        content_hash: String::new(),
                        model_version: String::new(),
                    },
                };
                tracing::info!(
                    chunk_id = %request.chunk_id,
                    text_len = chunk.text.len(),
                    "indexing chunk in vector store (no embedding provider configured; storing empty vector)"
                );
                if let Err(error) = adapters.vector_index.index_embeddings(vec![embedding]) {
                    tracing::error!(chunk_id = %request.chunk_id, %error, "failed to index vector");
                    return false;
                }
            }
            MaestriaEffect::UpdateGraph(request) => {
                let relation = {
                    let state = state.read().await;
                    state.relations.get(&request.relation_id).cloned()
                };
                let Some(relation) = relation else {
                    tracing::warn!(relation_id = %request.relation_id, "relation missing for graph update");
                    return true;
                };
                if let Err(error) = adapters.graph_index.insert_relation(relation) {
                    tracing::error!(relation_id = %request.relation_id, %error, "failed to insert relation into graph");
                    return false;
                }
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
            MaestriaEffect::FetchWeb(request) => match adapters.web_fetcher.fetch(&request.url) {
                Ok(snapshot) => {
                    tracing::debug!(
                        url = %request.url,
                        html_len = snapshot.html.len(),
                        "web fetch succeeded"
                    );
                }
                Err(error) => {
                    tracing::error!(url = %request.url, %error, "web fetch failed");
                    return false;
                }
            },
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

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{
        Artifact, ArtifactId, BlobId, Card, CardId, Chunk, ChunkId, DomainEventEnvelope, EventId,
        Evidence, EvidenceId, EvidenceKind, IndexStatus, LogicalTick, ParseArtifactRequest,
        SequenceNumber, StoreBlobRequest,
    };
    use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier};
    use maestria_ports::{
        CardRepository, ChunkRepository, EventFilter, EventLog, EvidenceRepository,
        InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
        InMemoryChunkRepository, InMemoryEventLog, InMemoryEvidenceRepository,
        InMemoryFullTextIndex, InMemoryGraphIndex, InMemoryHarnessAdapter, InMemoryParser,
        InMemoryVectorIndex, InMemoryWebFetcher, PortError,
    };
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    #[derive(Default)]
    struct FailingEventLog;

    impl EventLog for FailingEventLog {
        fn append(&self, _event: maestria_domain::DomainEventEnvelope) -> Result<(), PortError> {
            Err(PortError::Downstream {
                message: "event store unavailable".to_string(),
            })
        }

        fn scan(
            &self,
            _filter: EventFilter,
        ) -> Result<Vec<maestria_domain::DomainEventEnvelope>, PortError> {
            Ok(Vec::new())
        }
    }

    /// BlobStore that records every `put` payload for later assertion.
    struct RecordingBlobStore {
        recorded: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    }

    impl BlobStore for RecordingBlobStore {
        fn put(&self, bytes: Vec<u8>) -> Result<BlobId, PortError> {
            match self.recorded.lock() {
                Ok(mut guard) => {
                    let id = guard.len() as u64 + 1;
                    guard.push(bytes);
                    Ok(BlobId::new(id))
                }
                Err(_poisoned) => Err(PortError::Internal {
                    message: "recording blob store lock poisoned".to_string(),
                }),
            }
        }

        fn get(&self, _id: BlobId) -> Result<Vec<u8>, PortError> {
            Err(PortError::NotFound)
        }
    }

    #[tokio::test]
    async fn persist_effects_keep_duplicate_events_in_order() {
        let event_log = Arc::new(InMemoryEventLog::new());
        let adapters = Adapters {
            event_log: event_log.clone(),
            blob_store: Arc::new(InMemoryBlobStore::new()),
            search_index: Arc::new(InMemoryFullTextIndex::new()),
            harness: Arc::new(InMemoryHarnessAdapter::new()),
            parser: Arc::new(InMemoryParser::new()),
            artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
            chunk_repo: Arc::new(InMemoryChunkRepository::new()),
            card_repo: Arc::new(InMemoryCardRepository::new()),
            evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
            vector_index: Arc::new(InMemoryVectorIndex::new()),
            graph_index: Arc::new(InMemoryGraphIndex::new()),
            web_fetcher: Arc::new(InMemoryWebFetcher::new()),
        };
        let governance = Governance {
            classifier: Arc::new(DefaultRiskClassifier),
            approval_gate: Arc::new(DefaultApprovalGate),
        };
        let (runtime, input_rx) = MaestriaRuntime::new(
            RuntimeConfig {
                max_concurrent_effects: 2,
                default_effect_timeout: Duration::from_secs(2),
                max_retries: 0,
                ..RuntimeConfig::default()
            },
            KernelState::new(),
            adapters,
            governance,
        );
        let input_tx = runtime.handle().input_tx;
        let shutdown = CancellationToken::new();
        let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));

        input_tx
            .send(DomainInput::ClockTick(maestria_domain::LogicalTick::new(7)))
            .await
            .expect("first tick should be accepted");
        input_tx
            .send(DomainInput::ClockTick(maestria_domain::LogicalTick::new(7)))
            .await
            .expect("second tick should be accepted");

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if event_log
                    .scan(EventFilter { artifact_id: None })
                    .expect("event scan")
                    .len()
                    == 2
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("persist effects should complete");

        let events = event_log
            .scan(EventFilter { artifact_id: None })
            .expect("event scan");
        assert_eq!(events[0].id.value(), 1);
        assert_eq!(events[0].sequence.value(), 1);
        assert_eq!(events[1].id.value(), 2);
        assert_eq!(events[1].sequence.value(), 2);
        assert_eq!(events[0].event, events[1].event);

        shutdown.cancel();
        run.await.expect("runtime should shut down");
    }

    #[tokio::test]
    async fn failed_event_persistence_stops_runtime() {
        let adapters = Adapters {
            event_log: Arc::new(FailingEventLog),
            blob_store: Arc::new(InMemoryBlobStore::new()),
            search_index: Arc::new(InMemoryFullTextIndex::new()),
            harness: Arc::new(InMemoryHarnessAdapter::new()),
            parser: Arc::new(InMemoryParser::new()),
            artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
            chunk_repo: Arc::new(InMemoryChunkRepository::new()),
            card_repo: Arc::new(InMemoryCardRepository::new()),
            evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
            vector_index: Arc::new(InMemoryVectorIndex::new()),
            graph_index: Arc::new(InMemoryGraphIndex::new()),
            web_fetcher: Arc::new(InMemoryWebFetcher::new()),
        };
        let governance = Governance {
            classifier: Arc::new(DefaultRiskClassifier),
            approval_gate: Arc::new(DefaultApprovalGate),
        };
        let (runtime, input_rx) = MaestriaRuntime::new(
            RuntimeConfig {
                default_effect_timeout: Duration::from_secs(1),
                max_retries: 0,
                ..RuntimeConfig::default()
            },
            KernelState::new(),
            adapters,
            governance,
        );
        let input_tx = runtime.handle().input_tx;
        let shutdown = CancellationToken::new();
        let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));

        input_tx
            .send(DomainInput::ClockTick(maestria_domain::LogicalTick::new(1)))
            .await
            .expect("tick should be accepted before persistence failure");

        tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .expect("runtime should stop after fatal persistence failure")
            .expect("runtime task should join");
        assert!(shutdown.is_cancelled());
    }

    #[tokio::test]
    async fn store_blob_passes_exact_payload_bytes() {
        let recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
        let blob_store = Arc::new(RecordingBlobStore {
            recorded: recorded.clone(),
        });
        let adapters = Adapters {
            event_log: Arc::new(InMemoryEventLog::new()),
            blob_store,
            search_index: Arc::new(InMemoryFullTextIndex::new()),
            harness: Arc::new(InMemoryHarnessAdapter::new()),
            parser: Arc::new(InMemoryParser::new()),
            artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
            chunk_repo: Arc::new(InMemoryChunkRepository::new()),
            card_repo: Arc::new(InMemoryCardRepository::new()),
            evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
            vector_index: Arc::new(InMemoryVectorIndex::new()),
            graph_index: Arc::new(InMemoryGraphIndex::new()),
            web_fetcher: Arc::new(InMemoryWebFetcher::new()),
        };
        let governance = Governance {
            classifier: Arc::new(DefaultRiskClassifier),
            approval_gate: Arc::new(DefaultApprovalGate),
        };
        let (input_tx, _input_rx) = mpsc::channel(8);

        let expected = b"exact payload content".to_vec();
        let result = MaestriaRuntime::execute_effect(
            MaestriaEffect::StoreBlob(StoreBlobRequest {
                artifact_id: ArtifactId::new(1),
                payload: expected.clone(),
            }),
            Arc::new(adapters),
            Arc::new(governance),
            AutonomyProfile::TrustedWorkspace,
            Scope::default(),
            Arc::new(RwLock::new(KernelState::new())),
            input_tx,
        )
        .await;

        assert!(result, "StoreBlob should succeed");
        match recorded.lock() {
            Ok(guard) => {
                assert_eq!(guard.len(), 1, "exactly one blob stored");
                assert_eq!(guard[0], expected, "payload bytes preserved end to end");
            }
            Err(poisoned) => panic!("recording mutex poisoned: {:?}", poisoned),
        }
    }

    #[tokio::test]
    async fn parse_artifact_passes_exact_source_path_and_bytes() {
        let artifact_repo = InMemoryArtifactRepository::new();
        artifact_repo
            .put(Artifact {
                id: ArtifactId::new(42),
                title: "artifact-title-unused".to_string(),
                chunk_ids: BTreeSet::new(),
                card_ids: BTreeSet::new(),
                claim_ids: BTreeSet::new(),
                evidence_ids: BTreeSet::new(),
                index_status: IndexStatus::Unindexed,
                content_hash: None,
            })
            .expect("pre-populated artifact should be accepted");

        let adapters = Adapters {
            event_log: Arc::new(InMemoryEventLog::new()),
            blob_store: Arc::new(InMemoryBlobStore::new()),
            search_index: Arc::new(InMemoryFullTextIndex::new()),
            harness: Arc::new(InMemoryHarnessAdapter::new()),
            parser: Arc::new(InMemoryParser::new()),
            artifact_repo: Arc::new(artifact_repo),
            chunk_repo: Arc::new(InMemoryChunkRepository::new()),
            card_repo: Arc::new(InMemoryCardRepository::new()),
            evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
            vector_index: Arc::new(InMemoryVectorIndex::new()),
            graph_index: Arc::new(InMemoryGraphIndex::new()),
            web_fetcher: Arc::new(InMemoryWebFetcher::new()),
        };
        let governance = Governance {
            classifier: Arc::new(DefaultRiskClassifier),
            approval_gate: Arc::new(DefaultApprovalGate),
        };
        let (input_tx, mut input_rx) = mpsc::channel(8);

        let result = MaestriaRuntime::execute_effect(
            MaestriaEffect::ParseArtifact(ParseArtifactRequest {
                artifact_id: ArtifactId::new(42),
                source_path: "/repo/src/main.rs".to_string(),
                source_bytes: b"fn hello() {}".to_vec(),
            }),
            Arc::new(adapters),
            Arc::new(governance),
            AutonomyProfile::TrustedWorkspace,
            Scope::default(),
            Arc::new(RwLock::new(KernelState::new())),
            input_tx,
        )
        .await;

        assert!(result, "ParseArtifact should succeed");

        // First input: RecordEvidence for the single parsed chunk
        match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
            Ok(Some(DomainInput::RecordEvidence(ev))) => {
                assert_eq!(ev.artifact_id, ArtifactId::new(42));
                assert_eq!(ev.claim_id, None);
                assert_eq!(ev.observed_at, LogicalTick::new(1));
                match &ev.kind {
                    EvidenceKind::FileSpan {
                        path,
                        range,
                        content_hash,
                        snapshot,
                    } => {
                        assert_eq!(path, "/repo/src/main.rs");
                        assert_eq!(range.start, 1);
                        assert_eq!(range.end, 1);
                        assert!(content_hash.starts_with("sha256:"));
                        assert!(snapshot.is_some());
                    }
                    _ => panic!("expected FileSpan evidence, got {:?}", ev.kind),
                }
                assert!(!ev.excerpt.is_empty());
            }
            Ok(Some(other)) => panic!("expected RecordEvidence, got {other:?}"),
            Ok(None) => panic!("channel closed before RecordEvidence"),
            Err(_) => panic!("timeout waiting for RecordEvidence"),
        }

        // Second input: ParserCompleted
        match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
            Ok(Some(DomainInput::ParserCompleted(pr))) => {
                assert_eq!(pr.artifact_id, ArtifactId::new(42));
                assert_eq!(pr.chunks.len(), 1);
                assert_eq!(pr.chunks[0].text, "fn hello() {}");
            }
            Ok(Some(other)) => panic!("expected ParserCompleted, got {other:?}"),
            Ok(None) => panic!("channel closed before ParserCompleted"),
            Err(_) => panic!("timeout waiting for ParserCompleted"),
        }
    }

    #[tokio::test]
    async fn parse_artifact_empty_bytes_returns_failure() {
        let artifact_repo = InMemoryArtifactRepository::new();
        artifact_repo
            .put(Artifact {
                id: ArtifactId::new(7),
                title: "unused".to_string(),
                chunk_ids: BTreeSet::new(),
                card_ids: BTreeSet::new(),
                claim_ids: BTreeSet::new(),
                evidence_ids: BTreeSet::new(),
                index_status: IndexStatus::Unindexed,
                content_hash: None,
            })
            .expect("pre-populated artifact should be accepted");

        let adapters = Adapters {
            event_log: Arc::new(InMemoryEventLog::new()),
            blob_store: Arc::new(InMemoryBlobStore::new()),
            search_index: Arc::new(InMemoryFullTextIndex::new()),
            harness: Arc::new(InMemoryHarnessAdapter::new()),
            parser: Arc::new(InMemoryParser::new()),
            artifact_repo: Arc::new(artifact_repo),
            chunk_repo: Arc::new(InMemoryChunkRepository::new()),
            card_repo: Arc::new(InMemoryCardRepository::new()),
            evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
            vector_index: Arc::new(InMemoryVectorIndex::new()),
            graph_index: Arc::new(InMemoryGraphIndex::new()),
            web_fetcher: Arc::new(InMemoryWebFetcher::new()),
        };
        let governance = Governance {
            classifier: Arc::new(DefaultRiskClassifier),
            approval_gate: Arc::new(DefaultApprovalGate),
        };
        let (input_tx, _input_rx) = mpsc::channel(8);

        let result = MaestriaRuntime::execute_effect(
            MaestriaEffect::ParseArtifact(ParseArtifactRequest {
                artifact_id: ArtifactId::new(7),
                source_path: "/repo/empty.rs".to_string(),
                source_bytes: Vec::new(),
            }),
            Arc::new(adapters),
            Arc::new(governance),
            AutonomyProfile::TrustedWorkspace,
            Scope::default(),
            Arc::new(RwLock::new(KernelState::new())),
            input_tx,
        )
        .await;

        assert!(!result, "ParseArtifact with empty bytes should fail");
    }

    #[tokio::test]
    async fn persist_event_dispatches_chunk_card_evidence_to_repositories() {
        let artifact_id = ArtifactId::new(1);
        let chunk_id = ChunkId::new(1);
        let card_id = CardId::new(1);
        let evidence_id = EvidenceId::new(1);

        let artifact = Artifact {
            id: artifact_id,
            title: "test".into(),
            chunk_ids: [chunk_id].into(),
            card_ids: [card_id].into(),
            claim_ids: BTreeSet::new(),
            evidence_ids: [evidence_id].into(),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
        };
        let chunk = Chunk {
            id: chunk_id,
            artifact_id,
            order: 0,
            text: "chunk text".into(),
        };
        let card = Card {
            id: card_id,
            artifact_id,
            title: "card title".into(),
            body: "card body".into(),
            claim_ids: BTreeSet::new(),
        };
        let evidence = Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/test.txt".into(),
                range: maestria_domain::ContentRange { start: 0, end: 10 },
                content_hash: "abc".into(),
                snapshot: None,
            },
            excerpt: "excerpt".into(),
            observed_at: LogicalTick::new(1),
        };

        let mut state = KernelState::new();
        state.artifacts.insert(artifact_id, artifact);
        state.chunks.insert(chunk_id, chunk);
        state.cards.insert(card_id, card);
        state.evidences.insert(evidence_id, evidence);

        let chunk_repo = Arc::new(InMemoryChunkRepository::new());
        let card_repo = Arc::new(InMemoryCardRepository::new());
        let evidence_repo = Arc::new(InMemoryEvidenceRepository::new());
        let artifact_repo = Arc::new(InMemoryArtifactRepository::new());
        let event_log = Arc::new(InMemoryEventLog::new());

        let adapters = Adapters {
            event_log: event_log.clone(),
            blob_store: Arc::new(InMemoryBlobStore::new()),
            search_index: Arc::new(InMemoryFullTextIndex::new()),
            harness: Arc::new(InMemoryHarnessAdapter::new()),
            parser: Arc::new(InMemoryParser::new()),
            artifact_repo: artifact_repo.clone(),
            chunk_repo: chunk_repo.clone(),
            card_repo: card_repo.clone(),
            evidence_repo: evidence_repo.clone(),
            vector_index: Arc::new(InMemoryVectorIndex::new()),
            graph_index: Arc::new(InMemoryGraphIndex::new()),
            web_fetcher: Arc::new(InMemoryWebFetcher::new()),
        };
        let governance = Governance {
            classifier: Arc::new(DefaultRiskClassifier),
            approval_gate: Arc::new(DefaultApprovalGate),
        };
        let (input_tx, _input_rx) = mpsc::channel(8);

        let envelopes = vec![
            DomainEventEnvelope {
                id: EventId::new(1),
                sequence: SequenceNumber::new(1),
                event: DomainEvent::ChunkRegistered {
                    chunk_id,
                    artifact_id,
                    order: 0,
                    text: "chunk text".into(),
                },
            },
            DomainEventEnvelope {
                id: EventId::new(2),
                sequence: SequenceNumber::new(2),
                event: DomainEvent::CardCreated {
                    card_id,
                    artifact_id,
                    title: "card title".into(),
                    body: "card body".into(),
                },
            },
            DomainEventEnvelope {
                id: EventId::new(3),
                sequence: SequenceNumber::new(3),
                event: DomainEvent::EvidenceRecorded {
                    evidence_id,
                    artifact_id,
                    claim_id: None,
                    kind: EvidenceKind::FileSpan {
                        path: "/test.txt".into(),
                        range: maestria_domain::ContentRange { start: 0, end: 10 },
                        content_hash: "abc".into(),
                        snapshot: None,
                    },
                    excerpt: "excerpt".into(),
                    observed_at: LogicalTick::new(1),
                },
            },
        ];

        let adapters = Arc::new(adapters);
        let governance = Arc::new(governance);

        for envelope in &envelopes {
            let result = MaestriaRuntime::execute_effect(
                MaestriaEffect::PersistEvent {
                    envelope: envelope.clone(),
                },
                adapters.clone(),
                governance.clone(),
                AutonomyProfile::TrustedWorkspace,
                Scope::default(),
                Arc::new(RwLock::new(state.clone())),
                input_tx.clone(),
            )
            .await;
            assert!(result, "persist of {:?} should succeed", envelope.event);
        }

        assert!(
            chunk_repo.get(chunk_id).unwrap().is_some(),
            "chunk should be persisted"
        );
        assert!(
            card_repo.get(card_id).unwrap().is_some(),
            "card should be persisted"
        );
        assert!(
            evidence_repo.get(evidence_id).unwrap().is_some(),
            "evidence should be persisted"
        );
    }
}
