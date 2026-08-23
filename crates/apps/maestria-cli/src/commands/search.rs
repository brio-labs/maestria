use anyhow::{Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    DomainInput, EvidenceCandidate, SearchKnowledgeCompleted, SearchOutcome, SearchPlan, TaskId,
};
use std::path::PathBuf;

use crate::helpers;

/// Run an interactive search against the instance.
///
/// # Cancellation
/// Dropping this future tears down the CLI-side session (instance lock
/// released, runtime shutdown requested). A search already accepted by the
/// runtime may still reach durable state; inspect durable state before
/// retrying an interrupted command.
pub async fn run(
    instance_dir: PathBuf,
    task_id: Option<u64>,
    query: String,
    limit: usize,
) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;

    // Daemon-first: a live daemon serves searches from its warm runtime, so
    // the CLI skips the per-invocation state load entirely. Task-scoped
    // searches stay local — the daemon search operation has no task
    // binding. A missing daemon (no token/socket) falls back; a failing
    // daemon is a real error and surfaces.
    if task_id.is_none()
        && let Some(response) = try_daemon_search(&layout, &query, limit).await?
    {
        let store = maestria_storage_sqlite::SqliteStore::open_read_only(&layout.database_path)?;
        print_daemon_search(&store, &response);
        return Ok(());
    }

    let (_plan, outcome, _state) = run_search_command(&layout, task_id, query, limit).await?;
    let store = maestria_storage_sqlite::SqliteStore::open_read_only(&layout.database_path)?;
    print_search_outcome(&store, &outcome);
    Ok(())
}

/// Dispatch one search to a live instance daemon.
///
/// Returns `Ok(None)` when no daemon has ever started for this instance
/// (missing token or socket), so the caller runs the search locally. Once
/// the socket exists, daemon failures surface as errors: a daemon that is
/// running but failing is not a fallback trigger.
///
/// # Cancellation
///
/// Dropping this future closes the in-flight daemon request; a frame already
/// delivered to the daemon may still complete server-side.
async fn try_daemon_search(
    layout: &InstanceLayout,
    query: &str,
    limit: usize,
) -> Result<Option<maestria_daemon::SearchResponse>> {
    if !layout.system_dir.join("daemon.sock").exists() {
        return Ok(None);
    }
    let client = match maestria_daemon::DaemonClient::from_instance(layout) {
        Ok(client) => client,
        Err(_) => return Ok(None),
    };
    let response = match client
        .request(maestria_daemon::ClientOperation::Search {
            query: query.to_string(),
            limit,
        })
        .await
    {
        Ok(response) => response,
        Err(error) if error.code == maestria_daemon::ClientErrorCode::DaemonUnavailable => {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    match response {
        maestria_daemon::ClientResponse::Search(search) => Ok(Some(search)),
        _ => Err(anyhow!("daemon returned a non-search response")),
    }
}

/// Execute one governed search, persisting search knowledge through the runtime when the
/// instance write lock is available.
///
/// When another process holds the instance lock the search runs read-only and nothing is
/// persisted. When the lock is available, the command runs inside a `MutationSession`: the
/// session owns lock acquisition, reconciliation, and queued-recovery application, and the
/// completed search is submitted as a domain command so the runtime owns event persistence.
/// The durable/read-only decision is owned by `MutationSession::try_start_for_search`, which
/// acquires the instance lock exactly once.
pub(crate) async fn run_search_command(
    layout: &InstanceLayout,
    task_id: Option<u64>,
    query: String,
    limit: usize,
) -> Result<(SearchPlan, SearchOutcome, maestria_domain::KernelState)> {
    match maestria_daemon::MutationSession::try_start_for_search(
        layout.clone(),
        maestria_governance::AutonomyProfile::TrustedWorkspace,
    )
    .await?
    {
        Some(session) => run_durable_search(session, layout, task_id, query, limit).await,
        None => run_read_only_search(layout, task_id, query, limit).await,
    }
}

async fn run_durable_search(
    session: maestria_daemon::MutationSession,
    _layout: &InstanceLayout,
    task_id: Option<u64>,
    query: String,
    limit: usize,
) -> Result<(SearchPlan, SearchOutcome, maestria_domain::KernelState)> {
    let state = session.state().clone();
    let task_id = validate_task_id(&state, task_id)?;

    let result = async {
        // Reuse the session runtime's own search executor: the runtime already
        // assembled the retrieval graph (stores, indexes, policies) when it
        // started, so the durable path must not build a second one (R28).
        let executor = session
            .runtime_handle()
            .search_executor()
            .ok_or_else(|| anyhow!("runtime search executor is unavailable"))?;
        let (plan, outcome) = execute_search_with(&*executor, query, limit).await?;
        session
            .submit(DomainInput::SearchKnowledgeCompleted(
                SearchKnowledgeCompleted {
                    task_id,
                    plan: Box::new(plan.clone()),
                    outcome: outcome.clone(),
                },
            ))
            .await
            .map_err(|error| anyhow!("submit search knowledge: {error}"))?;
        Ok::<_, anyhow::Error>((plan, outcome))
    }
    .await;

    let (plan, outcome) = session.finish(result).await?;
    Ok((plan, outcome, state))
}
async fn run_read_only_search(
    layout: &InstanceLayout,
    task_id: Option<u64>,
    query: String,
    limit: usize,
) -> Result<(SearchPlan, SearchOutcome, maestria_domain::KernelState)> {
    let manifest = helpers::load_manifest(layout)?;
    // Task validation needs the tasks slice; everything else the read-only
    // assembly consumes is the generation registry, so plain searches skip
    // the full event-log replay.
    let (state, _task_id) = if task_id.is_some() {
        let state = helpers::load_kernel_state_with_retry(layout, "load kernel state for search")?;
        let task_id = validate_task_id(&state, task_id)?;
        (state, task_id)
    } else {
        let state = maestria_daemon::load_search_generations_state(layout)?;
        (state, None)
    };
    let (plan, outcome) = execute_search(layout, &state, &manifest, query, limit).await?;
    Ok((plan, outcome, state))
}

pub(crate) async fn execute_search(
    layout: &InstanceLayout,
    state: &maestria_domain::KernelState,
    manifest: &InstanceManifest,
    query: String,
    limit: usize,
) -> Result<(SearchPlan, SearchOutcome)> {
    let policy = maestria_governance::RetrievalSecurityPolicy::default()
        .require_read_allowed(true)
        .allow_unscoped_items(true);
    let runtime =
        maestria_daemon::prepare_search_runtime_read_only(layout, state, manifest, policy)?;
    execute_search_with(&*runtime, query, limit).await
}

/// Execute one governed search against an already-assembled executor and
/// verify the produced trace before returning.
///
/// `executor` is either the runtime-owned executor (durable path) or a
/// freshly assembled read-only search runtime (no runtime exists there, so
/// assembly is the single owner of that path).
async fn execute_search_with(
    executor: &dyn maestria_ports::SearchKnowledgeExecutor,
    query: String,
    limit: usize,
) -> Result<(SearchPlan, SearchOutcome)> {
    let (plan, outcome) = executor
        .plan_and_search(query, limit)
        .await
        .map_err(|error| anyhow!("search query execution: {error}"))?;
    let trace = outcome
        .trace_data
        .as_deref()
        .ok_or_else(|| anyhow!("search produced no durable trace payload"))?;
    outcome.verify_compatibility(&plan).map_err(|error| {
        anyhow!(
            "search produced an invalid trace for query `{}`: {error}",
            plan.original_query()
        )
    })?;
    if trace.deterministic_id() != outcome.trace || !trace.matches_plan(&plan) {
        return Err(anyhow!(
            "search produced a non-reproducible trace {}",
            outcome.trace
        ));
    }
    Ok((plan, outcome))
}

pub(crate) fn validate_task_id(
    state: &maestria_domain::KernelState,
    task_id: Option<u64>,
) -> Result<Option<TaskId>> {
    let Some(task_id) = task_id else {
        return Ok(None);
    };
    let task_id = TaskId::new(task_id);
    if !state.tasks.contains_key(&task_id) {
        anyhow::bail!("task {task_id} was not found");
    }
    Ok(Some(task_id))
}
pub(super) fn print_search_outcome(
    store: &maestria_storage_sqlite::SqliteStore,
    outcome: &SearchOutcome,
) {
    if outcome.evidence.is_empty() {
        println!("search_status={:?}", outcome.status);
        return;
    }
    for (rank, evidence_candidate) in outcome.evidence.iter().enumerate() {
        let (artifact_id, source, snippet) = describe_evidence(store, evidence_candidate);
        println!(
            "rank={} artifact={} evidence={} {} snippet={}",
            rank + 1,
            artifact_id,
            evidence_candidate.evidence_id(),
            source,
            snippet,
        );
    }
}

/// Render a daemon-served search with the same line format as the local
/// path. Hit details come from the same durable evidence projection the
/// daemon authorized against, read through this instance's store.
fn print_daemon_search(
    store: &maestria_storage_sqlite::SqliteStore,
    response: &maestria_daemon::SearchResponse,
) {
    use maestria_ports::EvidenceRepository;

    if response.evidence.is_empty() {
        println!("search_status={}", response.status);
        return;
    }
    for (rank, hit) in response.evidence.iter().enumerate() {
        let evidence_id = maestria_domain::EvidenceId::new(hit.evidence_id);
        let (artifact_label, source, snippet) = match EvidenceRepository::get(store, evidence_id) {
            Ok(Some(evidence)) => (
                evidence.artifact_id.to_string(),
                helpers::source_label(&evidence),
                sanitize_snippet(&evidence.excerpt),
            ),
            _ => (
                format!("artver:{}", hit.artifact_version),
                "source=missing".to_string(),
                "(missing evidence)".to_string(),
            ),
        };
        println!(
            "rank={} artifact={} evidence={} {} snippet={}",
            rank + 1,
            artifact_label,
            hit.evidence_id,
            source,
            snippet,
        );
    }
}

fn describe_evidence(
    store: &maestria_storage_sqlite::SqliteStore,
    candidate: &EvidenceCandidate,
) -> (String, String, String) {
    // Rendering reads the same durable evidence projection the retrieval
    // engine authorizes against, so it works on both the durable and the
    // replay-light read-only paths.
    let evidence = match maestria_ports::EvidenceRepository::get(store, candidate.evidence_id()) {
        Ok(Some(evidence)) => evidence,
        _ => {
            return (
                format!("artver:{}", candidate.artifact_version().value()),
                "source=missing".to_string(),
                "(missing evidence)".to_string(),
            );
        }
    };
    let source = helpers::source_label(&evidence);
    (
        evidence.artifact_id.to_string(),
        source,
        sanitize_snippet(&evidence.excerpt),
    )
}

fn sanitize_snippet(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}
