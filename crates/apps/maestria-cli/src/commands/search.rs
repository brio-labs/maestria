use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    DomainInput, EvidenceCandidate, SearchKnowledgeCompleted, SearchOutcome, SearchPlan, TaskId,
};
use std::{path::PathBuf, time::Duration};
use tokio::time::sleep;

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
    let write_mode = acquire_search_write_mode(&layout)?;
    let mut state = helpers::load_kernel_state_with_retry(
        &layout,
        Duration::from_secs(2),
        "load kernel state for search",
    )?;
    let task_id = validate_task_id(&state, task_id)?;
    let manifest = helpers::load_manifest(&layout)?;
    if write_mode.allows_persistence() {
        maestria_daemon::reconcile_retrieval_generations(&layout, &mut state, &manifest)
            .context("reconcile retrieval generations before search")?;
    }
    let (runtime, plan, outcome) = execute_search(
        &layout,
        &state,
        &manifest,
        query,
        limit,
        write_mode.allows_persistence(),
    )
    .await?;
    if write_mode.allows_persistence() {
        persist_search_knowledge_with_retry(&runtime, &mut state, task_id, &plan, &outcome).await?;
    }
    print_search_outcome(&state, &outcome);
    Ok(())
}

async fn persist_search_knowledge_with_retry(
    runtime: &maestria_daemon::SearchRuntime,
    state: &mut maestria_domain::KernelState,
    task_id: Option<TaskId>,
    plan: &SearchPlan,
    outcome: &SearchOutcome,
) -> Result<()> {
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match persist_search_knowledge(runtime, state, task_id, plan, outcome) {
                Ok(()) => return Ok(()),
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await;

    result.map_err(|_| anyhow!("timed out while recording search result"))?
}

pub(crate) async fn execute_search(
    layout: &InstanceLayout,
    state: &maestria_domain::KernelState,
    manifest: &InstanceManifest,
    query: String,
    limit: usize,
    allow_projection_writes: bool,
) -> Result<(
    std::sync::Arc<maestria_daemon::SearchRuntime>,
    SearchPlan,
    SearchOutcome,
)> {
    let policy = maestria_governance::RetrievalSecurityPolicy::default()
        .require_read_allowed(true)
        .allow_unscoped_items(true);
    let runtime = if allow_projection_writes {
        maestria_daemon::prepare_search_runtime(layout, state, manifest, policy)?
    } else {
        maestria_daemon::prepare_search_runtime_read_only(layout, state, manifest, policy)?
    };
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
    Ok((runtime, plan, outcome))
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

fn persist_search_knowledge(
    runtime: &maestria_daemon::SearchRuntime,
    state: &mut maestria_domain::KernelState,
    task_id: Option<TaskId>,
    plan: &SearchPlan,
    outcome: &SearchOutcome,
) -> Result<()> {
    let output = state.apply_input(DomainInput::SearchKnowledgeCompleted(
        SearchKnowledgeCompleted {
            task_id,
            plan: Some(Box::new(plan.clone())),
            outcome: outcome.clone(),
        },
    ))?;
    runtime.append_events(output.events)
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
