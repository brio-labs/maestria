use maestria_domain::{
    ApprovalDecision, Artifact, ArtifactId, BlobId, DomainEvent, DomainEventEnvelope, DomainInput,
    EvidenceKind, FullTextIndexCompleted, HarnessRunCompleted, IndexStatus, KernelState,
    LogicalTick, MaestriaEffect, ParserResult, ParserStarted, RecordEvidenceInput,
    RecordValidationReportInput, RegisterChunkInput, StartFullTextIndex, ValidationCompleted,
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
    collections::{BTreeMap, BTreeSet},
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

// ── shell grammar validation ──────────────────────────────────────────

/// Allowed shell commands for governed harness execution.
const ALLOWED_COMMANDS: &[&str] = &["echo", "pwd", "cat"];

/// Prohibited shell metacharacters and redirection operators.
const PROHIBITED_CHARS: &[char] = &[
    '|', '&', ';', '$', '`', '(', ')', '{', '}', '<', '>', '\\', '!', '~', '*', '?',
];

/// Returns `true` when `command` uses only the allowed grammar:
/// - starts with `echo`, `pwd`, or `cat`
/// - contains no shell metacharacters, redirection, or newlines
fn is_shell_grammar_allowed(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Reject embedded newlines
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return false;
    }

    // Reject prohibited metacharacters
    if trimmed.contains(PROHIBITED_CHARS) {
        return false;
    }

    // Must start with an allowed command word
    let first_word = trimmed.split_ascii_whitespace().next().map_or("", |w| w);
    ALLOWED_COMMANDS.contains(&first_word)
}

/// Extract the path arguments from a `cat` command.
/// Skips option tokens (leading `-`) and the `--` separator.
/// Returns empty vec for non-cat commands.
fn cat_path_args(command: &str) -> Vec<&str> {
    let trimmed = command.trim();
    let mut tokens = trimmed.split_ascii_whitespace();
    match tokens.next() {
        Some("cat") => tokens.filter(|t| !t.starts_with('-')).collect(),
        _ => Vec::new(),
    }
}

/// Determine a working directory that is contained within the configured scope.
/// Falls back to the current working directory when no roots are configured
/// (unrestricted scope).
fn resolve_working_directory(scope: &Scope) -> PathBuf {
    if let Some(root) = scope.readable_roots().first() {
        return root.clone();
    }
    // Scope with zero roots is unrestricted; use current directory.
    match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => PathBuf::from("."),
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
                        // PersistEvent must execute synchronously without consuming a
                        // semaphore permit. Otherwise a non-persist effect (e.g.
                        // ParseArtifact) holding the only permit at max_concurrent_effects=1
                        // would deadlock waiting for the PersistEvent it just enqueued.
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
                            if !success {
                                tracing::error!(
                                    "fatal event persistence failure; stopping runtime"
                                );
                                effect_shutdown.cancel();
                                break;
                            }
                            continue;
                        }

                        let Ok(permit) = semaphore.clone().acquire_owned().await else {
                            tracing::warn!("Effect executor semaphore closed");
                            break;
                        };
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
                    Some(context.default_effect_timeout),
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

    #[allow(clippy::too_many_arguments)]
    async fn execute_effect(
        effect: MaestriaEffect,
        adapters: Arc<Adapters>,
        governance: Arc<Governance>,
        profile: AutonomyProfile,
        configured_scope: Scope,
        state: Arc<RwLock<KernelState>>,
        input_tx: mpsc::Sender<DomainInput>,
        persistence_barrier_timeout: Option<Duration>,
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
                            if let Err(error) = adapters.evidence_repo.replace(evidence) {
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
            MaestriaEffect::StoreBlob(request) => {
                tracing::debug!(
                    artifact_id = %request.artifact_id,
                    "source blob queued for storage during parsing"
                );
            }
            MaestriaEffect::ParseArtifact(request) => {
                let artifact = match adapters.artifact_repo.get(request.artifact_id) {
                    Ok(Some(artifact)) => artifact,
                    Ok(None) => {
                        let state_read = state.read().await;
                        if let Some(artifact) =
                            state_read.artifacts.get(&request.artifact_id).cloned()
                        {
                            artifact
                        } else {
                            // Staged ingestion or resume: no persisted artifact yet. Construct an
                            // ephemeral typed parse context so the parser can proceed with the
                            // request metadata. The artifact is committed later by the domain
                            // handler when it receives ParserCompleted.
                            tracing::debug!(
                                artifact_id = %request.artifact_id,
                                "no persisted artifact; constructing ephemeral context for parse"
                            );
                            Artifact {
                                id: request.artifact_id,
                                title: request.source_path.clone(),
                                chunk_ids: BTreeSet::new(),
                                card_ids: BTreeSet::new(),
                                claim_ids: BTreeSet::new(),
                                evidence_ids: BTreeSet::new(),
                                index_status: IndexStatus::default(),
                                content_hash: None,
                            }
                        }
                    }
                    Err(error) => {
                        tracing::error!(artifact_id = %request.artifact_id, %error, "failed to load artifact for parse");
                        return false;
                    }
                };
                let path = PathBuf::from(&request.source_path);

                // Determine the bytes to parse and the blob identity.
                // - Fresh ingestion (source_blob is None): store bytes in the blob store
                //   exactly once and obtain an immutable BlobId.
                // - Resume (source_blob is Some): the blob already exists; fetch the exact
                //   bytes from the blob store to re-drive parsing.
                let (parse_bytes, blob_id, is_resume) = if let Some(blob_id) = request.source_blob {
                    match adapters.blob_store.get(blob_id) {
                        Ok(bytes) => (bytes, blob_id, true),
                        Err(error) => {
                            tracing::error!(
                                artifact_id = %artifact.id,
                                %blob_id,
                                %error,
                                "resume blob missing from store"
                            );
                            return false;
                        }
                    }
                } else {
                    match adapters.blob_store.put(request.source_bytes.clone()) {
                        Ok(blob_id) => (request.source_bytes.clone(), blob_id, false),
                        Err(error) => {
                            tracing::error!(
                                artifact_id = %artifact.id,
                                %error,
                                "failed to store source blob"
                            );
                            return false;
                        }
                    }
                };

                let metadata = FileMetadata {
                    path: path.clone(),
                    size: parse_bytes.len(),
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
                    return false;
                }

                let source_hash = content_hash(&parse_bytes);

                // On fresh ingestion, persist durable ParserStarted metadata so a crash
                // after this point leaves the parser request recoverable on restart.
                // Resume paths already have this event persisted; do not duplicate it.
                if !is_resume {
                    let title = {
                        let state_read = state.read().await;
                        state_read
                            .pending_artifacts
                            .get(&request.artifact_id)
                            .map_or_else(|| artifact.title.clone(), |p| p.title.clone())
                    };
                    Self::send_input(
                        &input_tx,
                        DomainInput::ParserStarted(ParserStarted {
                            artifact_id: artifact.id,
                            title,
                            source_path: request.source_path.clone(),
                            content_hash: source_hash.clone(),
                            blob_id,
                        }),
                        "parser started",
                    )
                    .await;

                    // Persistence barrier: wait until the ParserStarted event is
                    // observable in the event log before proceeding to parse. This
                    // closes the crash window where the parser could start before
                    // the durable resume marker is committed.
                    // Only active when the runtime path supplies a timeout (production);
                    // direct unit-test calls skip this via None.
                    if let Some(barrier_timeout) = persistence_barrier_timeout {
                        let capped = barrier_timeout.min(Duration::from_secs(30));
                        let persisted = Self::wait_for_parser_started_persistence(
                            &*adapters.event_log,
                            artifact.id,
                            blob_id,
                            &source_hash,
                            capped,
                        )
                        .await;
                        if !persisted {
                            tracing::error!(
                                artifact_id = %artifact.id,
                                "ParserStarted persistence barrier failed; not parsing"
                            );
                            return false;
                        }
                    }
                }

                // Compute the source text from the parse bytes before they are moved
                // into FileHandle. This avoids a second blob-store fetch on resume.
                let source_text = String::from_utf8_lossy(&parse_bytes).into_owned();

                let file = FileHandle {
                    path,
                    bytes: parse_bytes,
                };
                match adapters.parser.parse(
                    file,
                    ParseContext {
                        artifact_id: artifact.id,
                    },
                ) {
                    Ok(parsed) => {
                        let observed_at = LogicalTick::new(1);
                        let mut search_start: usize = 0;

                        // Collect evidence inputs and chunk registrations in a single pass.
                        let mut evidence_inputs: Vec<RecordEvidenceInput> = Vec::new();
                        let mut chunks: Vec<RegisterChunkInput> = Vec::new();
                        for (order, chunk) in parsed.chunks.iter().enumerate() {
                            let evidence_id = evidence_id_for(artifact.id, order as u32);
                            let range =
                                line_range_for_chunk(&source_text, &chunk.text, &mut search_start);
                            let excerpt = excerpt_for(&chunk.text);
                            evidence_inputs.push(RecordEvidenceInput {
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
                            });
                            chunks.push(RegisterChunkInput {
                                chunk_id: chunk.chunk_id,
                                artifact_id: chunk.artifact_id,
                                order: (order.min(u32::MAX as usize)) as u32,
                                text: chunk.text.clone(),
                            });
                        }
                        // Send ParserCompleted first so the domain handler commits the artifact
                        // (including ArtifactRegistered / PendingIndex) before evidence arrives.
                        // Evidence inputs follow after the artifact is guaranteed to exist in
                        // domain state. After evidence is persisted, StartFullTextIndex triggers
                        // full-text indexing via the domain's deferred gate.
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

                        for evidence in evidence_inputs {
                            Self::send_input(
                                &input_tx,
                                DomainInput::RecordEvidence(evidence),
                                "record evidence",
                            )
                            .await;
                        }

                        Self::send_input(
                            &input_tx,
                            DomainInput::StartFullTextIndex(StartFullTextIndex {
                                artifact_id: parsed.artifact_id,
                            }),
                            "start full-text index",
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

                // ── grammar restriction ──────────────────────────────────
                if !is_shell_grammar_allowed(&request.command) {
                    tracing::warn!(
                        command = %request.command,
                        "command violates shell grammar restrictions; not spawning"
                    );
                    return true;
                }

                // ── cat path containment ─────────────────────────────────
                if class == HarnessCommandClass::Shell && request.command.trim().starts_with("cat")
                {
                    for arg in cat_path_args(&request.command) {
                        let path = PathBuf::from(arg);
                        if let Err(containment_err) = scope.check_read_containment(&path) {
                            tracing::warn!(
                                path = %path.display(),
                                ?containment_err,
                                "cat path outside readable roots; not spawning"
                            );
                            return true;
                        }
                    }
                }

                // ── working directory ────────────────────────────────────
                let working_directory = resolve_working_directory(scope.scope());

                let harness_request = HarnessRequest {
                    run_id: request.run_id,
                    command: request.command.clone(),
                    working_directory,
                    duration_budget: Duration::from_secs(60),
                    class,
                    readable_roots: scope.readable_roots().to_vec(),
                };

                match adapters.harness.execute(harness_request).await {
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

    /// Polls the event log for a persisted ParserStarted envelope matching
    /// `artifact_id`, `blob_id`, _and_ `content_hash`. Returns `true` once
    /// the event is observable, or `false` on timeout / scan error.
    /// Uses deterministic backoff to avoid busy-waiting while the domain
    /// loop commits the event.
    async fn wait_for_parser_started_persistence(
        event_log: &dyn EventLog,
        artifact_id: ArtifactId,
        blob_id: BlobId,
        content_hash_val: &str,
        barrier_timeout: Duration,
    ) -> bool {
        // Scan all events — EventFilter artifact_id filtering may not cover
        // ParserStarted in every EventLog implementation.
        let contains_started = |entries: &[DomainEventEnvelope]| -> bool {
            entries.iter().any(|e| {
                matches!(
                    &e.event,
                    DomainEvent::ParserStarted {
                        artifact_id: id,
                        blob_id: bid,
                        content_hash: ch,
                        ..
                    } if *id == artifact_id
                        && *bid == blob_id
                        && ch == content_hash_val
                )
            })
        };

        // Immediate check without sleeping.
        match event_log.scan(EventFilter { artifact_id: None }) {
            Ok(events) if contains_started(&events) => return true,
            Err(error) => {
                tracing::error!(%error, "failed to scan event log for ParserStarted barrier");
                return false;
            }
            _ => {}
        }

        let deadline = tokio::time::Instant::now() + barrier_timeout;
        let mut backoff_ms: u64 = 1;

        loop {
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(
                    artifact_id = %artifact_id,
                    %blob_id,
                    timeout_ms = barrier_timeout.as_millis(),
                    "ParserStarted persistence barrier timed out; not parsing"
                );
                return false;
            }

            tokio::time::sleep(Duration::from_millis(backoff_ms.min(500))).await;
            backoff_ms = backoff_ms.saturating_mul(2);

            match event_log.scan(EventFilter { artifact_id: None }) {
                Ok(events) if contains_started(&events) => return true,
                Err(error) => {
                    tracing::error!(%error, "failed to scan event log during ParserStarted barrier");
                    return false;
                }
                _ => {}
            }
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

#[cfg(test)]
mod runtime_barrier_tests;
#[cfg(test)]
mod runtime_blob_tests;
#[cfg(test)]
mod runtime_evidence_tests;
#[cfg(test)]
mod runtime_harness_tests;
#[cfg(test)]
mod runtime_parse_tests;
#[cfg(test)]
mod runtime_resume_tests;
#[cfg(test)]
mod runtime_tests;
