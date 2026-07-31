use anyhow::{Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    DomainInput, EvidenceCandidate, SearchKnowledgeCompleted, SearchOutcome, SearchPlan, TaskId,
};
use std::{path::PathBuf, time::Duration};

use crate::helpers;

enum SearchWriteMode {
    Durable {
        _lock: maestria_daemon::InstanceWriteLock,
    },
    ReadOnly,
}

impl SearchWriteMode {
    fn allows_persistence(&self) -> bool {
        matches!(self, Self::Durable { .. })
    }
}

fn acquire_search_write_mode(layout: &InstanceLayout) -> Result<SearchWriteMode> {
    Ok(
        match maestria_daemon::try_acquire_instance_write_lock(layout)? {
            Some(lock) => SearchWriteMode::Durable { _lock: lock },
            None => SearchWriteMode::ReadOnly,
        },
    )
}

pub async fn run(
    instance_dir: PathBuf,
    task_id: Option<u64>,
    query: String,
    limit: usize,
) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let (_plan, outcome, state) = run_search_command(&layout, task_id, query, limit).await?;
    print_search_outcome(&state, &outcome);
    Ok(())
}

/// Execute one governed search, persisting search knowledge through the runtime when the
/// instance write lock is available.
///
/// When another process holds the instance lock the search runs read-only and nothing is
/// persisted. When the lock is available, the command runs inside a `MutationSession`: the
/// session owns lock acquisition, reconciliation, and queued-recovery application, and the
/// completed search is submitted as a domain command so the runtime owns event persistence.
pub(crate) async fn run_search_command(
    layout: &InstanceLayout,
    task_id: Option<u64>,
    query: String,
    limit: usize,
) -> Result<(SearchPlan, SearchOutcome, maestria_domain::KernelState)> {
    if acquire_search_write_mode(layout)?.allows_persistence() {
        run_durable_search(layout, task_id, query, limit).await
    } else {
        run_read_only_search(layout, task_id, query, limit).await
    }
}

async fn run_durable_search(
    layout: &InstanceLayout,
    task_id: Option<u64>,
    query: String,
    limit: usize,
) -> Result<(SearchPlan, SearchOutcome, maestria_domain::KernelState)> {
    let session = maestria_daemon::MutationSession::start_for_search(
        layout.clone(),
        maestria_governance::AutonomyProfile::TrustedWorkspace,
    )
    .await?;
    let state = session.state().clone();
    let task_id = validate_task_id(&state, task_id)?;
    let manifest = helpers::load_manifest(layout)?;

    let result = async {
        let (plan, outcome) = execute_search(layout, &state, &manifest, query, limit).await?;
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
    let state = helpers::load_kernel_state_with_retry(
        layout,
        Duration::from_secs(2),
        "load kernel state for search",
    )?;
    let _task_id = validate_task_id(&state, task_id)?;
    let manifest = helpers::load_manifest(layout)?;
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
    let (plan, outcome) = runtime.execute(query, limit).await?;
    let trace = outcome
        .trace_data
        .as_deref()
        .ok_or_else(|| anyhow!("search produced no durable trace payload"))?;
    outcome.verify_compatibility(&plan).map_err(|error| {
        anyhow!(
            "search produced an invalid trace for query `{}`: {error}",
            plan.original_query
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

pub(super) fn print_search_outcome(state: &maestria_domain::KernelState, outcome: &SearchOutcome) {
    if outcome.evidence.is_empty() {
        println!("search_status={:?}", outcome.status);
        return;
    }
    for (rank, evidence_candidate) in outcome.evidence.iter().enumerate() {
        let (artifact_id, source, snippet) = describe_evidence(state, evidence_candidate);
        println!(
            "rank={} artifact={} evidence={} {} snippet={}",
            rank + 1,
            artifact_id,
            evidence_candidate.evidence_id,
            source,
            snippet,
        );
    }
}

fn describe_evidence(
    state: &maestria_domain::KernelState,
    candidate: &EvidenceCandidate,
) -> (String, String, String) {
    let Some(evidence) = state.evidences.get(&candidate.evidence_id) else {
        return (
            format!("artver:{}", candidate.artifact_version.value()),
            "source=missing".to_string(),
            "(missing evidence)".to_string(),
        );
    };
    let source = helpers::source_label(evidence);
    (
        evidence.artifact_id.to_string(),
        source,
        sanitize_snippet(&evidence.excerpt),
    )
}

fn sanitize_snippet(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}
