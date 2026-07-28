use std::collections::BTreeSet;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, InstanceLayout, InstanceManifest, OpenEvidenceInput};
use maestria_domain::{
    ClaimId, DomainEvent, DomainInput, Evidence, EvidenceCandidate, EvidenceId, EvidenceKind,
    EvidenceSpan, HarnessRunCompleted, HarnessRunId, KernelState, MemoryCandidateId,
    RetrievalRawRank, RetrievalScoreKind, RetrievalScoreScale, ScopeId, SearchOutcome, Task,
    TaskId,
};
use maestria_governance::{PrivacyExclusions, ValidationRequest};
use maestria_parsers::ParserRegistry;
use maestria_ports::{
    ArtifactRepository, EffectJournalIntent, EffectJournalStatus, EvidenceRepository,
    HarnessRequest, ModelAgentProposal,
};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;

use super::server::ApiContext;
use super::{
    ClientOperation, ClientResponse, CoverageResponse, EvidenceResponse, EvidenceSourceResponse,
    ModelAgentHarnessOutcome, ModelAgentMemoryCandidateSummary, ModelAgentProposalPayload,
    ModelAgentProposalResponse, ModelAgentValidationSummary, SearchEvidenceResponse,
    SearchRawRankResponse, SearchResponse, SearchScoreResponse, SearchScoreScaleResponse,
    StatusResponse, TaskResponse, TaskSummary,
};

const MAX_SEARCH_LIMIT: usize = 100;
const DATABASE_RETRY_ATTEMPTS: usize = 80;
const DATABASE_RETRY_DELAY: Duration = Duration::from_millis(50);

pub(crate) async fn dispatch(
    context: &ApiContext,
    operation: ClientOperation,
) -> Result<ClientResponse> {
    match operation {
        ClientOperation::Status => {
            let layout = context.layout.clone();
            let socket_path = context.socket_path.clone();
            let response =
                run_database_retry("status", move || status(&layout, &socket_path)).await?;
            Ok(ClientResponse::Status(response))
        }
        ClientOperation::Task { task_id } => {
            let layout = context.layout.clone();
            let response = run_database_retry("task", move || task(&layout, task_id)).await?;
            Ok(ClientResponse::Task(response))
        }
        ClientOperation::Evidence { evidence_id } => {
            let layout = context.layout.clone();
            let response =
                run_database_retry("evidence", move || open_evidence(&layout, evidence_id)).await?;
            Ok(ClientResponse::Evidence(response))
        }
        ClientOperation::Search { query, limit } => {
            if query.trim().is_empty() {
                return Err(anyhow!("search query must not be empty"));
            }
            if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
                return Err(anyhow!(
                    "search limit must be between 1 and {MAX_SEARCH_LIMIT}"
                ));
            }
            Ok(ClientResponse::Search(
                search_with_retry(context, query, limit).await?,
            ))
        }
        ClientOperation::ModelAgentPropose { proposal } => {
            handle_model_agent_propose(context, proposal).await
        }
    }
}

fn current_generation(state: &KernelState) -> u64 {
    match state
        .event_log
        .iter()
        .filter_map(|env| match &env.event {
            DomainEvent::IndexGenerationStarted { id, .. } => Some(id.value()),
            _ => None,
        })
        .max()
    {
        Some(generation) => generation,
        None => {
            let _ = ();
            0
        }
    }
}

/// Converts the wire-format payload into a typed `ModelAgentProposal`.
///
/// Performs simple value wrapping (IDs, durations, paths) without validation.
fn build_proposal(payload: ModelAgentProposalPayload) -> ModelAgentProposal {
    let run_id = HarnessRunId::new(payload.run_id);
    let task_id = payload.task_id.map(TaskId::new);
    let evidence_ids: Vec<EvidenceId> = payload
        .evidence_ids
        .iter()
        .map(|id| EvidenceId::new(*id))
        .collect();
    let working_directory = std::path::PathBuf::from(&payload.working_directory);
    let timeout = Duration::from_secs(payload.timeout_secs);

    ModelAgentProposal {
        run_id,
        task_id,
        query: payload.query,
        limit: payload.limit,
        capability: payload.capability,
        command: payload.command,
        working_directory,
        timeout,
        expected_generation: payload.expected_generation,
        evidence_ids,
    }
}

/// Validates a proposal against the current kernel state and generation.
///
/// Checks that the expected generation matches the latest index generation and
/// that all referenced evidence IDs exist in the current state. Returns a
/// `GovernedAgentProposal` on success, or an error with context if validation fails.
fn validate_proposal_against_state(
    proposal: &ModelAgentProposal,
    state: &KernelState,
) -> Result<maestria_ports::GovernedAgentProposal> {
    let cur_gen = current_generation(state);
    let available_evidence: BTreeSet<EvidenceId> = state.evidences.keys().copied().collect();

    proposal
        .validate(cur_gen, &available_evidence)
        .map_err(|error| anyhow!("proposal validation failed: {error}"))
}

/// Accumulated side-effects produced by executing a model-agent proposal.
///
/// Required effects either complete or return a typed [`ProposalEffectFailure`]. Warnings are
/// reserved for non-fatal validation diagnostics.
struct ProposalEffects {
    trace_id: Option<u64>,
    evidence_count: usize,
    harness: Option<ModelAgentHarnessOutcome>,
    validation: Option<ModelAgentValidationSummary>,
    memory_candidate: Option<ModelAgentMemoryCandidateSummary>,
    warnings: Vec<String>,
}

#[derive(Debug)]
enum ProposalEffectFailure {
    Search { message: String },
    Harness { message: String },
    CompletionDelivery { message: String },
    CandidateDelivery { message: String },
    Journal { message: String },
}

impl std::fmt::Display for ProposalEffectFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Search { message } => write!(formatter, "search effect failed: {message}"),
            Self::Harness { message } => write!(formatter, "harness effect failed: {message}"),
            Self::CompletionDelivery { message } => {
                write!(formatter, "harness completion delivery failed: {message}")
            }
            Self::CandidateDelivery { message } => {
                write!(formatter, "memory candidate delivery failed: {message}")
            }
            Self::Journal { message } => write!(formatter, "effect journal failure: {message}"),
        }
    }
}

impl std::error::Error for ProposalEffectFailure {}

/// Orchestrates the side-effects of a proposal: search, harness, validation, and memory promotion.
///
/// Ordering invariant: search runs first (if a query is present), then harness execution
/// (if a command is present), then validation, and finally memory-candidate creation.
///
/// Required effect failures abort the workflow with a typed error. Harness failures terminalize
/// the durable effect journal as `Failed`; failures delivering feedback or a memory candidate
/// pause the harness generation so recovery can resume it.
async fn execute_proposal_effects(
    context: &ApiContext,
    proposal: &ModelAgentProposal,
    governed: &maestria_ports::GovernedAgentProposal,
    state: &KernelState,
) -> std::result::Result<ProposalEffects, ProposalEffectFailure> {
    let mut trace_id = None;
    let mut evidence_count = 0usize;
    if !governed.search_query.trim().is_empty() {
        let (_plan, outcome) = search_knowledge(context, governed).await.map_err(|error| {
            ProposalEffectFailure::Search {
                message: error.to_string(),
            }
        })?;
        trace_id = Some(outcome.trace.value());
        evidence_count = outcome.evidence.len();
    }

    let (harness, harness_generation) = if !governed.harness.command.trim().is_empty() {
        let (outcome, generation) = execute_governed_harness(context, proposal, governed).await?;
        let completion = HarnessRunCompleted {
            run_id: governed.harness.run_id,
            generation,
            task_id: proposal.task_id,
            command: governed.harness.command.clone(),
            exit_code: outcome.exit_code,
            output: String::from_utf8_lossy(&outcome.stdout).to_string(),
        };
        if let Err(error) = context
            .input_tx
            .send(DomainInput::HarnessRunCompleted(completion))
            .await
        {
            let failure = ProposalEffectFailure::CompletionDelivery {
                message: format!("channel closed ({error})"),
            };
            return Err(terminalize_harness_failure(
                context,
                governed.harness.run_id,
                generation,
                EffectJournalStatus::Paused,
                failure,
            ));
        }
        (
            Some(ModelAgentHarnessOutcome {
                exit_code: outcome.exit_code,
                stdout: truncate_utf8(&outcome.stdout, 4096),
                stderr: truncate_utf8(&outcome.stderr, 4096),
                duration_ms: outcome.duration.as_millis() as u64,
            }),
            Some(generation),
        )
    } else {
        (None, None)
    };

    let (validation, validation_warning) = evaluate_validation_gate(context, proposal, state);
    let mut warnings = Vec::new();
    if let Some(warning) = validation_warning {
        warnings.push(warning);
    }
    let memory_candidate =
        create_memory_candidate(context, governed, state, &harness, harness_generation).await?;

    Ok(ProposalEffects {
        trace_id,
        evidence_count,
        harness,
        validation,
        memory_candidate,
        warnings,
    })
}

fn evaluate_validation_gate(
    context: &ApiContext,
    proposal: &ModelAgentProposal,
    state: &KernelState,
) -> (Option<ModelAgentValidationSummary>, Option<String>) {
    let Some(task_id) = proposal.task_id else {
        return (None, None);
    };
    let Some(task) = state.tasks.get(&task_id) else {
        return (
            None,
            Some("referenced task not found in kernel state".into()),
        );
    };
    let request = ValidationRequest {
        task: task.clone(),
        validation_report: None,
        proposed_status: maestria_domain::TaskStatus::CompletedVerified,
    };
    let summary = match context.governance.validation_gate.evaluate(&request) {
        maestria_governance::ValidationDecision::AllowCompletion => {
            Some(ModelAgentValidationSummary {
                passed: true,
                warnings: Vec::new(),
            })
        }
        _ => Some(ModelAgentValidationSummary {
            passed: false,
            warnings: vec!["validation gate did not allow completion".into()],
        }),
    };
    (summary, None)
}

async fn create_memory_candidate(
    context: &ApiContext,
    governed: &maestria_ports::GovernedAgentProposal,
    state: &KernelState,
    harness: &Option<ModelAgentHarnessOutcome>,
    harness_generation: Option<u64>,
) -> std::result::Result<Option<ModelAgentMemoryCandidateSummary>, ProposalEffectFailure> {
    if harness.is_none() || governed.evidence_ids.is_empty() {
        return Ok(None);
    }
    let candidate_id = MemoryCandidateId::new(
        state
            .memory_candidates
            .keys()
            .map(|id| id.value())
            .fold(0, u64::max)
            + 1,
    );
    let candidate = maestria_domain::MemoryCandidate {
        id: candidate_id,
        claim_id: ClaimId::new(1),
        evidence_ids: governed.evidence_ids.iter().copied().collect(),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let request = maestria_governance::MemoryPromotionRequest {
        candidate: candidate.clone(),
        user_approved: false,
    };
    let decision = context.governance.memory_promotion_gate.evaluate(&request);
    let decision_str = match &decision {
        maestria_governance::MemoryPromotionDecision::Promote => "promote",
        maestria_governance::MemoryPromotionDecision::RequireEvidence { .. } => "require_evidence",
        maestria_governance::MemoryPromotionDecision::RequireReview { .. } => "require_review",
        maestria_governance::MemoryPromotionDecision::Deny { .. } => "deny",
    };
    match context
        .input_tx
        .send(DomainInput::CreateMemoryCandidate(
            maestria_domain::CreateMemoryCandidateInput {
                candidate_id,
                claim_id: ClaimId::new(1),
                evidence_ids: governed.evidence_ids.clone(),
                confidence_milli: 800,
                security: None,
            },
        ))
        .await
    {
        Ok(()) => Ok(Some(ModelAgentMemoryCandidateSummary {
            candidate_id: candidate_id.value(),
            confidence_milli: 800,
            decision: decision_str.to_string(),
        })),
        Err(error) => {
            let failure = ProposalEffectFailure::CandidateDelivery {
                message: format!("channel closed ({error})"),
            };
            match harness_generation {
                Some(generation) => Err(terminalize_harness_failure(
                    context,
                    governed.harness.run_id,
                    generation,
                    EffectJournalStatus::Paused,
                    failure,
                )),
                None => Err(failure),
            }
        }
    }
}

async fn handle_model_agent_propose(
    context: &ApiContext,
    payload: ModelAgentProposalPayload,
) -> Result<ClientResponse> {
    let proposal = build_proposal(payload);
    let state = crate::load_kernel_state(&context.layout)
        .with_context(|| "load kernel state for proposal validation")?;
    let governed = validate_proposal_against_state(&proposal, &state)?;
    let effects = execute_proposal_effects(context, &proposal, &governed, &state).await?;

    Ok(ClientResponse::ModelAgentProposal(
        ModelAgentProposalResponse {
            run_id: proposal.run_id.value(),
            trace_id: effects.trace_id,
            index_generation: current_generation(&state),
            evidence_count: effects.evidence_count,
            harness: effects.harness,
            validation: effects.validation,
            memory_candidate: effects.memory_candidate,
            warnings: effects.warnings,
        },
    ))
}

async fn prepare_read_only_search_runtime(
    context: &ApiContext,
) -> Result<Arc<crate::SearchRuntime>> {
    let layout = context.layout.clone();
    let (state, manifest) = tokio::task::spawn_blocking(move || load_state_and_manifest(&layout))
        .await
        .map_err(|error| anyhow!("load search state task failed: {error}"))??;
    let layout = context.layout.clone();
    let runtime = tokio::task::spawn_blocking(move || {
        crate::prepare_search_runtime_read_only(
            &layout,
            &state,
            &manifest,
            maestria_governance::RetrievalSecurityPolicy::default()
                .require_read_allowed(true)
                .allow_unscoped_items(true),
        )
    })
    .await
    .map_err(|error| anyhow!("prepare search runtime task failed: {error}"))??;
    Ok(runtime)
}

async fn search_knowledge(
    context: &ApiContext,
    governed: &maestria_ports::GovernedAgentProposal,
) -> Result<(maestria_domain::SearchPlan, maestria_domain::SearchOutcome)> {
    let runtime = prepare_read_only_search_runtime(context).await?;
    runtime
        .execute(governed.search_query.clone(), governed.search_limit)
        .await
}

async fn execute_governed_harness(
    context: &ApiContext,
    proposal: &ModelAgentProposal,
    governed: &maestria_ports::GovernedAgentProposal,
) -> std::result::Result<(maestria_ports::HarnessOutcome, u64), ProposalEffectFailure> {
    let harness = &context.adapters.harness;
    let capabilities = harness
        .capabilities()
        .map_err(|error| ProposalEffectFailure::Harness {
            message: format!("harness capabilities: {error}"),
        })?;

    if !capabilities
        .command_classes
        .contains(&governed.harness.class)
    {
        return Err(ProposalEffectFailure::Harness {
            message: format!(
                "harness adapter does not support capability {:?}",
                governed.harness.class
            ),
        });
    }

    let command = governed.harness.command.trim();
    if command.is_empty() {
        return Err(ProposalEffectFailure::Harness {
            message: "harness command must not be empty".to_string(),
        });
    }
    let allowed_commands = ["echo", "pwd", "cat"];
    let Some(first_word) = command.split_ascii_whitespace().next() else {
        return Err(ProposalEffectFailure::Harness {
            message: "harness command must contain a command name".to_string(),
        });
    };
    if !allowed_commands.contains(&first_word) {
        return Err(ProposalEffectFailure::Harness {
            message: format!("command not in allowed set: {first_word}"),
        });
    }
    let prohibited_chars = &[
        '|', '&', ';', '$', '`', '(', ')', '{', '}', '<', '>', '\\', '!', '~', '*', '?',
    ];
    if command.contains(prohibited_chars) {
        return Err(ProposalEffectFailure::Harness {
            message: "command contains prohibited shell metacharacters".to_string(),
        });
    }

    let scope = harness_scope(context, &governed.harness.working_directory).map_err(|error| {
        ProposalEffectFailure::Harness {
            message: error.to_string(),
        }
    })?;

    let request = HarnessRequest {
        run_id: governed.harness.run_id,
        command: command.to_string(),
        working_directory: governed.harness.working_directory.clone(),
        duration_budget: governed.harness.duration_budget,
        class: governed.harness.class.clone(),
        readable_roots: scope.readable_roots().to_vec(),
        blocked_paths: scope.blocked_paths().to_vec(),
        blocked_patterns: scope.blocked_patterns().to_vec(),
    };

    let generation = record_harness_start(context, proposal, governed, command)?;
    let outcome = match harness.execute(request).await {
        Ok(outcome) => outcome,
        Err(error) => {
            let failure = ProposalEffectFailure::Harness {
                message: format!("harness execution failed: {error}"),
            };
            return Err(terminalize_harness_failure(
                context,
                governed.harness.run_id,
                generation,
                EffectJournalStatus::Failed,
                failure,
            ));
        }
    };
    if let Err(error) = context
        .adapters
        .effect_journal
        .claim_feedback(governed.harness.run_id, generation)
    {
        let failure = ProposalEffectFailure::Harness {
            message: format!("failed to claim harness feedback: {error}"),
        };
        return Err(terminalize_harness_failure(
            context,
            governed.harness.run_id,
            generation,
            EffectJournalStatus::Failed,
            failure,
        ));
    }

    Ok((outcome, generation))
}

fn harness_scope(
    context: &ApiContext,
    working_directory: &std::path::Path,
) -> Result<maestria_governance::Scope> {
    let manifest = InstanceManifest::decode(&fs::read_to_string(&context.layout.manifest_path)?)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    let working_directory = working_directory.canonicalize().with_context(|| {
        format!(
            "canonicalize working directory {}",
            working_directory.display()
        )
    })?;
    let roots = manifest
        .read_roots
        .iter()
        .map(|root| {
            root.canonicalize()
                .with_context(|| format!("canonicalize manifest read root {}", root.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    if roots.is_empty() {
        return Err(anyhow!("no valid read roots in manifest"));
    }
    if !roots.iter().any(|root| working_directory.starts_with(root)) {
        return Err(anyhow!(
            "working directory {} is outside instance read roots",
            working_directory.display()
        ));
    }
    Ok(
        maestria_governance::Scope::new(roots, vec![], vec!["shell".into()], vec![], false)
            .with_blocked_patterns(runtime_blocked_patterns(&manifest)),
    )
}

fn runtime_blocked_patterns(manifest: &InstanceManifest) -> Vec<String> {
    let default_privacy = PrivacyExclusions::default();
    let mut blocked_patterns = manifest.excluded_patterns.clone();
    blocked_patterns.extend(default_privacy.sensitive_names().iter().cloned());
    blocked_patterns.extend(
        default_privacy
            .sensitive_extensions()
            .iter()
            .map(|extension| format!("*.{extension}")),
    );
    blocked_patterns
}

fn record_harness_start(
    context: &ApiContext,
    proposal: &ModelAgentProposal,
    governed: &maestria_ports::GovernedAgentProposal,
    command: &str,
) -> std::result::Result<u64, ProposalEffectFailure> {
    let entry = context
        .adapters
        .effect_journal
        .record_intent(EffectJournalIntent {
            run_id: governed.harness.run_id,
            task_id: proposal.task_id,
            capability: "shell".to_string(),
            command: command.to_string(),
            scope_id: maestria_domain::ScopeId::new(1),
            requested_generation: None,
        })
        .map_err(|error| ProposalEffectFailure::Harness {
            message: format!("failed to record harness intent: {error}"),
        })?;
    if let Err(error) = context
        .adapters
        .effect_journal
        .record_started(governed.harness.run_id, entry.generation)
    {
        let failure = ProposalEffectFailure::Harness {
            message: format!("failed to record harness started: {error}"),
        };
        return Err(terminalize_harness_failure(
            context,
            governed.harness.run_id,
            entry.generation,
            EffectJournalStatus::Failed,
            failure,
        ));
    }
    Ok(entry.generation)
}

fn terminalize_harness_failure(
    context: &ApiContext,
    run_id: HarnessRunId,
    generation: u64,
    status: EffectJournalStatus,
    failure: ProposalEffectFailure,
) -> ProposalEffectFailure {
    match context
        .adapters
        .effect_journal
        .record_terminal(run_id, generation, status)
    {
        Ok(()) => failure,
        Err(error) => ProposalEffectFailure::Journal {
            message: format!(
                "{failure}; failed to record harness status {status:?} for run {run_id} generation \
                 {generation}: {error}"
            ),
        },
    }
}

fn status(layout: &InstanceLayout, socket_path: &std::path::Path) -> Result<StatusResponse> {
    let state = load_state(layout)?;
    Ok(StatusResponse {
        instance_root: layout.root.display().to_string(),
        event_count: state.event_log.len(),
        task_count: state.tasks.len(),
        socket_path: socket_path.display().to_string(),
    })
}

fn task(layout: &InstanceLayout, task_id: Option<u64>) -> Result<TaskResponse> {
    let state = load_state(layout)?;
    let tasks: Vec<TaskSummary> = state
        .tasks
        .iter()
        .filter(|(id, _)| task_id.is_none_or(|requested| id.value() == requested))
        .map(|(_, task)| task_summary(task))
        .collect();
    if task_id.is_some() && tasks.is_empty() {
        return Err(anyhow!("task not found"));
    }
    Ok(TaskResponse { tasks })
}

async fn run_database_retry<T, F>(operation_name: &str, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: Fn() -> Result<T> + Send + Sync + 'static,
{
    let operation = Arc::new(operation);
    for attempt in 0..DATABASE_RETRY_ATTEMPTS {
        let op = Arc::clone(&operation);
        let result = tokio::task::spawn_blocking(move || op())
            .await
            .map_err(|error| anyhow!("{operation_name} task failed: {error}"))?;
        match result {
            Ok(response) => return Ok(response),
            Err(error) if is_database_locked(&error) && attempt + 1 < DATABASE_RETRY_ATTEMPTS => {
                tokio::time::sleep(DATABASE_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(anyhow!("{operation_name} retries exhausted"))
}

async fn search_with_retry(
    context: &ApiContext,
    query: String,
    limit: usize,
) -> Result<SearchResponse> {
    for attempt in 0..DATABASE_RETRY_ATTEMPTS {
        match search(context, query.clone(), limit).await {
            Ok(response) => return Ok(response),
            Err(error) if is_database_locked(&error) && attempt + 1 < DATABASE_RETRY_ATTEMPTS => {
                tokio::time::sleep(DATABASE_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(anyhow!("search query retries exhausted"))
}

async fn search(context: &ApiContext, query: String, limit: usize) -> Result<SearchResponse> {
    let runtime = prepare_read_only_search_runtime(context).await?;
    let (plan, outcome) = runtime.execute(query, limit).await?;
    Ok(search_response(
        plan.original_query,
        plan.query_id.value(),
        outcome,
    ))
}

fn is_database_locked(error: &anyhow::Error) -> bool {
    let rendered = format!("{error:#}");
    rendered.contains("locked") || rendered.contains("busy")
}

fn open_evidence(layout: &InstanceLayout, evidence_id: u64) -> Result<EvidenceResponse> {
    let manifest = InstanceManifest::decode(&fs::read_to_string(&layout.manifest_path)?)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    let sqlite = SqliteStore::open(&layout.database_path)?;
    let evidence_id = maestria_domain::EvidenceId::new(evidence_id);
    let retrieval_policy = maestria_governance::RetrievalSecurityPolicy::default()
        .require_read_allowed(true)
        .required_scope(ScopeId::new(1))
        .allow_unscoped_items(true);
    if let Some(evidence) = EvidenceRepository::get(&sqlite, evidence_id)? {
        validate_evidence_scope(&manifest, &evidence)?;
        if !matches!(
            retrieval_policy.evaluate(&evidence.security),
            maestria_governance::RetrievalDecision::Allowed
        ) {
            return Err(anyhow!(
                "evidence is not available: not available under retrieval policy"
            ));
        }
        if let Some(artifact) = ArtifactRepository::get(&sqlite, evidence.artifact_id)?
            && !matches!(
                retrieval_policy.evaluate(&artifact.security),
                maestria_governance::RetrievalDecision::Allowed
            )
        {
            return Err(anyhow!(
                "artifact is not available: not available under retrieval policy"
            ));
        }
    }
    let blobs = FsBlobStore::open(&layout.blobs_dir)?;
    let search_index = TantivyFullTextIndex::open_read_only(&layout.full_text_index_dir)?;
    let parser = ParserRegistry::with_defaults();
    let core = CoreServices::new(CorePorts {
        artifacts: &sqlite,
        chunks: &sqlite,
        cards: &sqlite,
        evidence: &sqlite,
        events: &sqlite,
        parser: &parser,
        search_index: &search_index,
        blobs: &blobs,
        vector_index: None,
        graph_index: None,
    })
    .with_retrieval_policy(retrieval_policy);
    let output = core.open_evidence(OpenEvidenceInput { evidence_id })?;
    Ok(EvidenceResponse {
        evidence_id: output.evidence.id.value(),
        artifact_id: output.artifact.id.value(),
        artifact_title: output.artifact.title,
        artifact_content_hash: output.artifact.content_hash,
        source: evidence_source(&output.evidence)?,
        excerpt: output.evidence.excerpt,
        observed_at: output.evidence.observed_at.value(),
    })
}

fn validate_evidence_scope(manifest: &InstanceManifest, evidence: &Evidence) -> Result<()> {
    let EvidenceKind::FileSpan { path, .. } = &evidence.kind else {
        return Ok(());
    };
    if source_scope_allowed(manifest, path) {
        return Ok(());
    }
    Err(anyhow!(
        "evidence source path {} is outside instance read roots or excluded by policy",
        path
    ))
}

fn source_scope_allowed(manifest: &InstanceManifest, path: &str) -> bool {
    let path = std::path::Path::new(path);
    let mut candidates = vec![lexical_normalize(path)];
    if path.is_relative() {
        candidates.push(lexical_normalize(&manifest.root.join(path)));
    }
    let roots: Vec<_> = manifest
        .read_roots
        .iter()
        .map(|root| lexical_normalize(root))
        .collect();
    let blocked_patterns = runtime_blocked_patterns(manifest);
    candidates.iter().any(|candidate| {
        roots.iter().any(|root| candidate.starts_with(root))
            && !blocked_patterns
                .iter()
                .any(|pattern| path_matches_pattern(candidate, pattern))
    })
}

fn lexical_normalize(path: &std::path::Path) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn path_matches_pattern(path: &std::path::Path, pattern: &str) -> bool {
    path.components()
        .any(|component| glob_matches(&component.as_os_str().to_string_lossy(), pattern))
}

fn glob_matches(value: &str, pattern: &str) -> bool {
    let value: Vec<char> = value.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let mut value_index = 0usize;
    let mut pattern_index = 0usize;
    let mut star_pattern_index = None;
    let mut star_value_index = 0usize;

    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == '?' || pattern[pattern_index] == value[value_index])
        {
            value_index += 1;
            pattern_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            star_pattern_index = Some(pattern_index);
            star_value_index = value_index;
            pattern_index += 1;
        } else if let Some(star_index) = star_pattern_index {
            star_value_index += 1;
            value_index = star_value_index;
            pattern_index = star_index + 1;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

fn load_state(layout: &InstanceLayout) -> Result<KernelState> {
    crate::load_kernel_state(layout)
}

fn load_state_and_manifest(layout: &InstanceLayout) -> Result<(KernelState, InstanceManifest)> {
    let state = load_state(layout)?;
    let manifest = InstanceManifest::decode(&fs::read_to_string(&layout.manifest_path)?)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    Ok((state, manifest))
}

fn search_response(query: String, query_id: u64, outcome: SearchOutcome) -> SearchResponse {
    SearchResponse {
        query,
        query_id,
        trace_id: outcome.trace.value(),
        status: format!("{:?}", outcome.status),
        fingerprint: outcome.fingerprint.as_str().to_string(),
        index_generation: outcome.index_generation.value(),
        evidence: outcome.evidence.iter().map(search_evidence).collect(),
        coverage: CoverageResponse {
            percent_covered: outcome.coverage.percent_covered,
            gaps: outcome.coverage.gaps_identified,
            distinct_sources: outcome.coverage.distinct_sources,
            distinct_documents: outcome.coverage.distinct_documents,
            distinct_sections: outcome.coverage.distinct_sections,
        },
        conflict_count: outcome.conflicts.len(),
    }
}

fn search_evidence(candidate: &EvidenceCandidate) -> SearchEvidenceResponse {
    SearchEvidenceResponse {
        evidence_id: candidate.evidence_id.value(),
        artifact_version: candidate.artifact_version.value(),
        source: format_source_span(&candidate.source_span),
        range_start: candidate.source_span.range().start,
        range_end: candidate.source_span.range().end,
        score_schema_version: candidate.scores.schema_version,
        scores: candidate.scores.lanes.iter().map(search_score).collect(),
        trust: format!("{:?}", candidate.trust),
        freshness: format!("{:?}", candidate.freshness),
    }
}

fn search_score(score: &maestria_domain::RetrievalLaneScore) -> SearchScoreResponse {
    SearchScoreResponse {
        score_kind: score_kind_name(&score.score_kind),
        raw_score: score.raw_score,
        raw_rank: match &score.raw_rank {
            RetrievalRawRank::Ranked { rank } => SearchRawRankResponse::Ranked { rank: *rank },
            RetrievalRawRank::Unavailable { reason } => SearchRawRankResponse::Unavailable {
                reason: reason.clone(),
            },
        },
        scale: match &score.scale {
            RetrievalScoreScale::Binary => SearchScoreScaleResponse::Binary,
            RetrievalScoreScale::Unbounded {
                name,
                higher_is_better,
            } => SearchScoreScaleResponse::Unbounded {
                name: name.clone(),
                higher_is_better: *higher_is_better,
            },
            RetrievalScoreScale::FixedPoint {
                name,
                denominator,
                minimum,
                maximum,
                higher_is_better,
            } => SearchScoreScaleResponse::FixedPoint {
                name: name.clone(),
                denominator: *denominator,
                minimum: *minimum,
                maximum: *maximum,
                higher_is_better: *higher_is_better,
            },
            RetrievalScoreScale::RankDerived {
                name,
                higher_is_better,
            } => SearchScoreScaleResponse::RankDerived {
                name: name.clone(),
                higher_is_better: *higher_is_better,
            },
        },
        representation: score.representation.0.clone(),
        fingerprint: score.fingerprint.identity.as_str().to_string(),
        fingerprint_components: score.fingerprint.components.clone(),
    }
}

fn score_kind_name(kind: &RetrievalScoreKind) -> String {
    match kind {
        RetrievalScoreKind::Exact => "exact".to_string(),
        RetrievalScoreKind::LexicalBm25 => "lexical_bm25".to_string(),
        RetrievalScoreKind::DenseSimilarity => "dense_similarity".to_string(),
        RetrievalScoreKind::LearnedSparse => "learned_sparse".to_string(),
        RetrievalScoreKind::LateInteraction => "late_interaction".to_string(),
        RetrievalScoreKind::Graph => "graph".to_string(),
        RetrievalScoreKind::SpecializedRetrieval { route } => {
            format!("specialized_retrieval:{route}")
        }
    }
}

fn format_source_span(span: &EvidenceSpan) -> String {
    match span.location() {
        maestria_domain::SourceLocation::File {
            path,
            start_line,
            end_line,
        } => format!("{path}:{start_line}-{end_line}"),
        maestria_domain::SourceLocation::Page {
            page_start,
            page_end,
        } => format!("pages {page_start}-{page_end}"),
        maestria_domain::SourceLocation::Region {
            page,
            x,
            y,
            width,
            height,
        } => format!("page {page} region {x},{y} {width}x{height}"),
        maestria_domain::SourceLocation::Symbol {
            path,
            qualified_name,
        } => format!("{path}::{qualified_name}"),
    }
}

fn evidence_source(evidence: &Evidence) -> Result<EvidenceSourceResponse> {
    Ok(match &evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            content_hash,
            ..
        } => EvidenceSourceResponse::File {
            path: path.clone(),
            start_line: u32::try_from(range.start)
                .context("file evidence start line exceeds u32")?,
            end_line: u32::try_from(range.end).context("file evidence end line exceeds u32")?,
            content_hash: content_hash.clone(),
        },
        EvidenceKind::PdfSpan {
            blob,
            page_start,
            page_end,
        } => EvidenceSourceResponse::Pdf {
            snapshot_id: blob.value(),
            page_start: *page_start,
            page_end: *page_end,
        },
        EvidenceKind::PdfRegion {
            blob,
            page,
            x,
            y,
            width,
            height,
        } => EvidenceSourceResponse::PdfRegion {
            snapshot_id: blob.value(),
            page: *page,
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        },
        EvidenceKind::WebSnapshot {
            url,
            snapshot,
            content_hash,
            ..
        } => EvidenceSourceResponse::Web {
            url: url.clone(),
            content_hash: content_hash.clone(),
            snapshot_id: snapshot.value(),
        },
        EvidenceKind::CommandOutput {
            harness_run,
            stream,
            blob,
        } => EvidenceSourceResponse::Command {
            harness_run: harness_run.value(),
            stream: format!("{stream:?}"),
            blob_id: blob.value(),
        },
        EvidenceKind::TestResult {
            harness_run,
            status,
            log,
        } => EvidenceSourceResponse::Test {
            harness_run: harness_run.value(),
            status: format!("{status:?}"),
            log_id: log.value(),
        },
        EvidenceKind::Diff {
            harness_run,
            patch_blob,
        } => EvidenceSourceResponse::Diff {
            harness_run: harness_run.value(),
            patch_blob_id: patch_blob.value(),
        },
        EvidenceKind::Validation { report_id } => EvidenceSourceResponse::Validation {
            report_id: report_id.value(),
        },
    })
}

fn task_summary(task: &Task) -> TaskSummary {
    TaskSummary {
        task_id: task.id.value(),
        title: task.title.clone(),
        status: format!("{:?}", task.status),
        priority: format!("{:?}", task.priority),
        evidence_ids: task.evidence_ids.iter().map(|id| id.value()).collect(),
        validation_report_id: task.validation_report_id.map(|id| id.value()),
    }
}

fn truncate_utf8(bytes: &[u8], max_len: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= max_len {
        return text.into_owned();
    }
    if max_len <= 3 {
        return "...".chars().take(max_len).collect();
    }
    let prefix_limit = max_len - 3;
    let prefix_end = text.char_indices().fold(0, |end, (index, character)| {
        let candidate = index + character.len_utf8();
        if candidate <= prefix_limit {
            candidate
        } else {
            end
        }
    });
    let mut truncated = String::with_capacity(prefix_end + 3);
    truncated.push_str(&text[..prefix_end]);
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
#[path = "services_tests.rs"]
mod tests;
