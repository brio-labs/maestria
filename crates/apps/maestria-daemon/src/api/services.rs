use std::collections::BTreeSet;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, InstanceLayout, InstanceManifest, OpenEvidenceInput};
use maestria_domain::{
    ApprovalDecision, ApprovalId, DomainEvent, DomainInput, Evidence, EvidenceCandidate,
    EvidenceId, EvidenceKind, EvidenceSpan, HarnessRunId, KernelState, ModelAgentProposalExecution,
    RetrievalRawRank, RetrievalScoreKind, RetrievalScoreScale, ScopeId, SearchOutcome, Task,
    TaskId,
};
use maestria_governance::PrivacyExclusions;
use maestria_parsers::ParserRegistry;
use maestria_ports::{
    ApprovalRecord, ApprovalRepository, ArtifactRepository, EffectJournal, EvidenceRepository,
    ModelAgentProposal,
};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;

use super::server::ApiContext;
use super::{
    ClientOperation, ClientResponse, CoverageResponse, EvidenceResponse, EvidenceSourceResponse,
    ModelAgentProposalPayload, ModelAgentProposalResponse, ModelAgentStatusResponse,
    SearchEvidenceResponse, SearchRawRankResponse, SearchResponse, SearchScoreResponse,
    SearchScoreScaleResponse, StatusResponse, TaskResponse, TaskSummary,
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
        ClientOperation::ModelAgentStatus { run_id } => {
            let layout = context.layout.clone();
            let response = run_database_retry("model-agent status", move || {
                model_agent_status(&layout, run_id)
            })
            .await?;
            Ok(ClientResponse::ModelAgentStatus(response))
        }
        ClientOperation::ModelAgentResolve {
            run_id,
            approval_id,
            approved,
        } => handle_model_agent_resolution(context, run_id, approval_id, approved).await,
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

#[derive(Debug, serde::Deserialize)]
struct PendingHarnessContinuation {
    proposal: maestria_domain::ModelAgentProposalRequest,
    journal_generation: u64,
    correlation_id: u64,
}

#[derive(Debug, Clone, Copy)]
struct PendingProposalIdentity {
    run_id: u64,
    correlation_id: u64,
    journal_generation: u64,
}

fn decode_pending_continuation(record: &ApprovalRecord) -> Option<PendingHarnessContinuation> {
    if record.effect_kind != "model_agent_harness" {
        return None;
    }
    let token = record.capability.strip_prefix("model_agent_pending:")?;
    let continuation = serde_json::from_str::<PendingHarnessContinuation>(token).ok()?;
    match &continuation.proposal.execution {
        ModelAgentProposalExecution::ApprovalContinuation {
            approval_id,
            journal_generation,
        } if *approval_id == record.id
            && *journal_generation == continuation.journal_generation =>
        {
            Some(continuation)
        }
        ModelAgentProposalExecution::Fresh
        | ModelAgentProposalExecution::JournalRecovery { .. }
        | ModelAgentProposalExecution::ApprovalContinuation { .. } => None,
    }
}

fn pending_proposal_identity(record: &ApprovalRecord) -> Option<PendingProposalIdentity> {
    let continuation = decode_pending_continuation(record)?;
    let journal_generation = continuation.proposal.execution.journal_generation()?;
    Some(PendingProposalIdentity {
        run_id: continuation.proposal.run_id.value(),
        correlation_id: continuation.correlation_id,
        journal_generation,
    })
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
        .map_err(anyhow::Error::new)
}

async fn handle_model_agent_propose(
    context: &ApiContext,
    payload: ModelAgentProposalPayload,
) -> Result<ClientResponse> {
    let task_validation = payload.task_validation;
    let memory_candidate = payload.memory_candidate;
    let proposal = build_proposal(payload);
    let state = crate::instance_setup::load_kernel_state(&context.layout)
        .with_context(|| "load kernel state for proposal validation")?;
    validate_proposal_against_state(&proposal, &state)?;
    let Some(runtime) = context.runtime.clone() else {
        return Err(anyhow!(
            "model-agent proposal requires the canonical runtime command path"
        ));
    };
    let result = runtime
        .submit(DomainInput::ModelAgentProposalRequested(
            maestria_domain::ModelAgentProposalRequest {
                run_id: proposal.run_id,
                task_id: proposal.task_id,
                query: proposal.query,
                limit: proposal.limit,
                evidence_ids: proposal.evidence_ids.clone(),
                capability: proposal.capability,
                command: proposal.command,
                working_directory: proposal.working_directory.display().to_string(),
                timeout_secs: proposal.timeout.as_secs(),
                expected_generation: proposal.expected_generation,
                task_validation,
                memory_candidate,
                execution: ModelAgentProposalExecution::Fresh,
                correlation_id: 0,
            },
        ))
        .await
        .map_err(|error| anyhow!("model-agent proposal was not accepted: {error}"))?;

    Ok(ClientResponse::ModelAgentProposal(
        ModelAgentProposalResponse {
            run_id: proposal.run_id.value(),
            correlation_id: result.correlation_id,
            status: "accepted".to_string(),
            approval_id: None,
            trace_id: None,
            index_generation: current_generation(&state),
            evidence_count: proposal.evidence_ids.len(),
            harness: None,
            validation: None,
            memory_candidate: None,
            warnings: vec![format!(
                "runtime accepted proposal correlation {} with deferred query, validation, \
                 memory, and harness outcomes; use model_agent_status",
                result.correlation_id
            )],
        },
    ))
}

async fn handle_model_agent_resolution(
    context: &ApiContext,
    run_id: u64,
    approval_id: u64,
    approved: bool,
) -> Result<ClientResponse> {
    let store = SqliteStore::open(&context.layout.database_path)?;
    let record = store
        .find_by_id(ApprovalId::new(approval_id))?
        .ok_or_else(|| anyhow!("model-agent approval {approval_id} does not exist"))?;
    let identity = pending_proposal_identity(&record)
        .ok_or_else(|| anyhow!("approval {approval_id} is not a model-agent proposal"))?;
    let pending_run_id = identity.run_id;
    let correlation_id = identity.correlation_id;
    if pending_run_id != run_id {
        return Err(anyhow!(
            "approval {approval_id} belongs to model-agent run {pending_run_id}, not {run_id}"
        ));
    }
    let Some(runtime) = context.runtime.clone() else {
        return Err(anyhow!(
            "model-agent approval requires the canonical runtime command path"
        ));
    };
    runtime
        .submit(DomainInput::ApprovalResolved(ApprovalDecision {
            approval_id: record.id,
            task_id: record.task_id,
            approved,
            affects_task: false,
        }))
        .await
        .map_err(|error| anyhow!("model-agent approval was not accepted: {error}"))?;
    Ok(ClientResponse::ModelAgentStatus(ModelAgentStatusResponse {
        run_id,
        correlation_id: Some(correlation_id),
        status: if approved {
            "approval_recorded"
        } else {
            "denial_recorded"
        }
        .to_string(),
        approval_id: Some(approval_id),
        journal_generation: None,
        trace_id: None,
        evidence_count: 0,
        harness: None,
        validation: None,
        memory_candidate: None,
        error: None,
    }))
}

fn model_agent_status(layout: &InstanceLayout, run_id: u64) -> Result<ModelAgentStatusResponse> {
    let state = crate::instance_setup::load_kernel_state(layout)
        .map_err(|error| anyhow!("load model-agent terminal result state: {error:#}"))?;
    if let Some(result) = state
        .model_agent_results
        .get(&maestria_domain::HarnessRunId::new(run_id))
    {
        return Ok(ModelAgentStatusResponse {
            run_id,
            correlation_id: Some(result.correlation_id),
            status: match result.status {
                maestria_domain::ModelAgentTerminalStatus::Succeeded => "succeeded",
                maestria_domain::ModelAgentTerminalStatus::Failed => "failed",
            }
            .to_string(),
            approval_id: None,
            journal_generation: None,
            trace_id: result.search.as_ref().map(|search| search.trace_id),
            evidence_count: result
                .search
                .as_ref()
                .map_or(0, |search| search.evidence_count),
            harness: result
                .harness
                .as_ref()
                .map(|harness| super::ModelAgentHarnessOutcome {
                    exit_code: harness.exit_code,
                    stdout: harness.stdout.clone(),
                    stderr: harness.stderr.clone(),
                    duration_ms: harness.duration_ms,
                }),
            validation: result.validation.as_ref().map(|validation| {
                super::ModelAgentValidationSummary {
                    passed: validation.passed,
                    warnings: validation.warnings.clone(),
                }
            }),
            memory_candidate: result.memory_candidate.as_ref().map(|memory| {
                super::ModelAgentMemoryCandidateSummary {
                    candidate_id: memory.candidate_id.value(),
                    confidence_milli: memory.confidence_milli,
                    decision: memory.decision.as_str().to_string(),
                }
            }),
            error: result.error.clone(),
        });
    }
    let store = SqliteStore::open(&layout.database_path)?;
    let mut approval_id = None;
    let mut correlation_id = None;
    let mut pending_journal_generation = None;
    for record in store.find_pending()? {
        let Some(identity) = pending_proposal_identity(&record) else {
            continue;
        };
        if identity.run_id == run_id {
            approval_id = Some(record.id.value());
            correlation_id = Some(identity.correlation_id);
            pending_journal_generation = Some(identity.journal_generation);
            break;
        }
    }
    let journal = store.scan_in_flight()?;
    let entry = journal.iter().find(|entry| {
        entry.run_id.value() == run_id
            && pending_journal_generation.is_none_or(|generation| entry.generation == generation)
    });
    let status = if approval_id.is_some() {
        "pending_approval"
    } else if entry.is_some() {
        "running"
    } else {
        "submitted"
    };
    Ok(ModelAgentStatusResponse {
        run_id,
        correlation_id,
        status: status.to_string(),
        approval_id,
        journal_generation: pending_journal_generation
            .or_else(|| entry.map(|entry| entry.generation)),
        trace_id: None,
        evidence_count: 0,
        harness: None,
        validation: None,
        memory_candidate: None,
        error: None,
    })
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

async fn prepare_read_only_search_runtime(
    context: &ApiContext,
) -> Result<Arc<crate::SearchRuntime>> {
    let layout = context.layout.clone();
    let (state, manifest) = tokio::task::spawn_blocking(move || load_state_and_manifest(&layout))
        .await
        .map_err(|error| anyhow!("load search state task failed: {error}"))??;
    let layout = context.layout.clone();
    tokio::task::spawn_blocking(move || {
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
    .map_err(|error| anyhow!("prepare search runtime task failed: {error}"))?
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
        if let maestria_governance::RetrievalDecision::Denied(reason) =
            retrieval_policy.evaluate(&evidence.security)
        {
            return Err(anyhow!(
                "evidence is not available under retrieval policy: {reason}"
            ));
        }
        validate_evidence_scope(&manifest, &evidence)?;
        if let Some(artifact) = ArtifactRepository::get(&sqlite, evidence.artifact_id)?
            && let maestria_governance::RetrievalDecision::Denied(reason) =
                retrieval_policy.evaluate(&artifact.security)
        {
            return Err(anyhow!(
                "artifact is not available under retrieval policy: {reason}"
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
    crate::instance_setup::load_kernel_state(layout)
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
            snapshot,
        } => EvidenceSourceResponse::File {
            path: path.clone(),
            start_line: u32::try_from(range.start())
                .context("file evidence start line exceeds u32")?,
            end_line: u32::try_from(range.end()).context("file evidence end line exceeds u32")?,
            content_hash: snapshot.content_hash().as_str().to_string(),
        },
        EvidenceKind::PdfSpan {
            snapshot,
            page_start,
            page_end,
        } => EvidenceSourceResponse::Pdf {
            snapshot_id: snapshot.blob_id().value(),
            page_start: *page_start,
            page_end: *page_end,
        },
        EvidenceKind::PdfRegion {
            snapshot,
            page,
            x,
            y,
            width,
            height,
        } => EvidenceSourceResponse::PdfRegion {
            snapshot_id: snapshot.blob_id().value(),
            page: *page,
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        },
        EvidenceKind::WebSnapshot { url, snapshot, .. } => EvidenceSourceResponse::Web {
            url: url.clone(),
            content_hash: snapshot.content_hash().as_str().to_string(),
            snapshot_id: snapshot.blob_id().value(),
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

#[cfg(test)]
#[path = "services_tests.rs"]
mod tests;
