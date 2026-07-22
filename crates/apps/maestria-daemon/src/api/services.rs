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
    RetrievalRawRank, RetrievalScoreKind, RetrievalScoreScale, SearchOutcome, Task, TaskId,
};
use maestria_governance::{ScopeGuard, ValidationRequest};
use maestria_parsers::ParserRegistry;
use maestria_ports::{HarnessRequest, ModelAgentProposal};
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
    state
        .event_log
        .iter()
        .filter_map(|env| match &env.event {
            DomainEvent::IndexGenerationStarted { id, .. } => Some(id.value()),
            _ => None,
        })
        .max()
        .map_or(0, |generation| generation)
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
/// Each field is optional because individual effect steps may be skipped or fail
/// without aborting the overall operation. Warnings collect non-fatal errors.
struct ProposalEffects {
    trace_id: Option<u64>,
    evidence_count: usize,
    harness: Option<ModelAgentHarnessOutcome>,
    validation: Option<ModelAgentValidationSummary>,
    memory_candidate: Option<ModelAgentMemoryCandidateSummary>,
    warnings: Vec<String>,
}

/// Orchestrates the side-effects of a proposal: search, harness, validation, and memory promotion.
///
/// Ordering invariant: search runs first (if a query is present), then harness execution
/// (if a command is present), then validation, and finally memory-candidate creation.
///
/// Error handling: each step is independent; failures are logged as warnings and do not
/// abort subsequent steps. The harness step additionally emits a `HarnessRunCompleted`
/// domain event on success.
async fn execute_proposal_effects(
    context: &ApiContext,
    proposal: &ModelAgentProposal,
    governed: &maestria_ports::GovernedAgentProposal,
    state: &KernelState,
) -> ProposalEffects {
    let mut warnings = Vec::new();

    let mut trace_id = None;
    let mut evidence_count = 0usize;
    if !governed.search_query.trim().is_empty() {
        match search_knowledge(context, governed).await {
            Ok((_plan, outcome)) => {
                trace_id = Some(outcome.trace.value());
                evidence_count = outcome.evidence.len();
            }
            Err(error) => {
                warnings.push(format!("search step warning: {error}"));
            }
        }
    }

    let harness = if !governed.harness.command.trim().is_empty() {
        match execute_governed_harness(context, governed).await {
            Ok(outcome) => {
                let completion = HarnessRunCompleted {
                    run_id: governed.harness.run_id,
                    generation: current_generation(state),
                    task_id: proposal.task_id,
                    command: governed.harness.command.clone(),
                    exit_code: outcome.exit_code,
                    output: String::from_utf8_lossy(&outcome.stdout).to_string(),
                };
                let _ = context
                    .input_tx
                    .try_send(DomainInput::HarnessRunCompleted(completion));
                Some(ModelAgentHarnessOutcome {
                    exit_code: outcome.exit_code,
                    stdout: truncate_utf8(&outcome.stdout, 4096),
                    stderr: truncate_utf8(&outcome.stderr, 4096),
                    duration_ms: outcome.duration.as_millis() as u64,
                })
            }
            Err(e) => {
                warnings.push(format!("harness execution warning: {e}"));
                None
            }
        }
    } else {
        None
    };

    let (validation, validation_warning) = evaluate_validation_gate(context, proposal, state);
    if let Some(warning) = validation_warning {
        warnings.push(warning);
    }
    let memory_candidate = create_memory_candidate(context, governed, state, &harness);

    ProposalEffects {
        trace_id,
        evidence_count,
        harness,
        validation,
        memory_candidate,
        warnings,
    }
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

fn create_memory_candidate(
    context: &ApiContext,
    governed: &maestria_ports::GovernedAgentProposal,
    state: &KernelState,
    harness: &Option<ModelAgentHarnessOutcome>,
) -> Option<ModelAgentMemoryCandidateSummary> {
    if harness.is_none() || governed.evidence_ids.is_empty() {
        return None;
    }
    let candidate_id = MemoryCandidateId::new(
        state
            .memory_candidates
            .keys()
            .map(|id| id.value())
            .max()
            .map_or(0, |candidate_id| candidate_id)
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
    let _ = context
        .input_tx
        .try_send(DomainInput::CreateMemoryCandidate(
            maestria_domain::CreateMemoryCandidateInput {
                candidate_id,
                claim_id: ClaimId::new(1),
                evidence_ids: governed.evidence_ids.clone(),
                confidence_milli: 800,
                security: None,
            },
        ));
    Some(ModelAgentMemoryCandidateSummary {
        candidate_id: candidate_id.value(),
        confidence_milli: 800,
        decision: decision_str.to_string(),
    })
}

async fn handle_model_agent_propose(
    context: &ApiContext,
    payload: ModelAgentProposalPayload,
) -> Result<ClientResponse> {
    let proposal = build_proposal(payload);
    let state = crate::load_kernel_state(&context.layout)
        .with_context(|| "load kernel state for proposal validation")?;
    let governed = validate_proposal_against_state(&proposal, &state)?;
    let effects = execute_proposal_effects(context, &proposal, &governed, &state).await;

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

async fn search_knowledge(
    context: &ApiContext,
    governed: &maestria_ports::GovernedAgentProposal,
) -> Result<(maestria_domain::SearchPlan, maestria_domain::SearchOutcome)> {
    let layout_a = context.layout.clone();
    let (state, manifest) = tokio::task::spawn_blocking(move || load_state_and_manifest(&layout_a))
        .await
        .map_err(|error| anyhow!("load search state: {error}"))?
        .map_err(|error| anyhow!("load search state: {error}"))?;
    let layout_b = context.layout.clone();
    let runtime = tokio::task::spawn_blocking(move || {
        crate::prepare_search_runtime_read_only(
            &layout_b,
            &state,
            &manifest,
            maestria_governance::RetrievalSecurityPolicy::default()
                .require_read_allowed(true)
                .allow_unscoped_items(true),
        )
    })
    .await
    .map_err(|error| anyhow!("prepare search runtime: {error}"))?
    .map_err(|error| anyhow!("prepare search runtime: {error}"))?;
    runtime
        .execute(governed.search_query.clone(), governed.search_limit)
        .await
}

async fn execute_governed_harness(
    context: &ApiContext,
    governed: &maestria_ports::GovernedAgentProposal,
) -> Result<maestria_ports::HarnessOutcome> {
    let harness = &context.adapters.harness;
    let capabilities = harness
        .capabilities()
        .map_err(|e| anyhow!("harness capabilities: {e}"))?;

    if !capabilities
        .command_classes
        .contains(&governed.harness.class)
    {
        return Err(anyhow!(
            "harness adapter does not support capability {:?}",
            governed.harness.class
        ));
    }

    let command = governed.harness.command.trim();
    if command.is_empty() {
        return Err(anyhow!("harness command must not be empty"));
    }
    let allowed_commands = ["echo", "pwd", "cat"];
    let Some(first_word) = command.split_ascii_whitespace().next() else {
        return Err(anyhow!("harness command must contain a command name"));
    };
    if !allowed_commands.contains(&first_word) {
        return Err(anyhow!("command not in allowed set: {first_word}"));
    }
    let prohibited_chars = &[
        '|', '&', ';', '$', '`', '(', ')', '{', '}', '<', '>', '\\', '!', '~', '*', '?',
    ];
    if command.contains(prohibited_chars) {
        return Err(anyhow!("command contains prohibited shell metacharacters"));
    }

    let current_directory = std::env::current_dir().context("resolve daemon working directory")?;
    let scope_guard = ScopeGuard::new(maestria_governance::Scope::new(
        vec![current_directory],
        vec![],
        vec!["shell".into()],
        vec![],
        false,
    ));
    let scope = scope_guard.scope();

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

    harness
        .execute(request)
        .await
        .map_err(|e| anyhow!("harness execution failed: {e}"))
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
    let layout_a = context.layout.clone();
    let (state, manifest) = tokio::task::spawn_blocking(move || load_state_and_manifest(&layout_a))
        .await
        .map_err(|error| anyhow!("load search state task failed: {error}"))??;
    let layout_b = context.layout.clone();
    let runtime = tokio::task::spawn_blocking(move || {
        crate::prepare_search_runtime_read_only(
            &layout_b,
            &state,
            &manifest,
            maestria_governance::RetrievalSecurityPolicy::default()
                .require_read_allowed(true)
                .allow_unscoped_items(true),
        )
    })
    .await
    .map_err(|error| anyhow!("prepare search runtime task failed: {error}"))??;
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
    let sqlite = SqliteStore::open(&layout.database_path)?;
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
    });
    let output = core.open_evidence(OpenEvidenceInput {
        evidence_id: maestria_domain::EvidenceId::new(evidence_id),
    })?;
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
    let s = String::from_utf8_lossy(bytes);
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}
