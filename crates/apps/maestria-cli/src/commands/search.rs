use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    DomainInput, EvidenceCandidate, SearchKnowledgeCompleted, SearchOutcome, SearchPlan, TaskId,
};
use std::path::PathBuf;

use crate::helpers;

pub async fn run(
    instance_dir: PathBuf,
    task_id: Option<u64>,
    query: String,
    limit: usize,
) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let _instance_lock = maestria_daemon::acquire_instance_write_lock(&layout).await?;
    let mut state =
        maestria_daemon::load_kernel_state(&layout).context("load kernel state for search")?;
    let task_id = validate_task_id(&state, task_id)?;
    let manifest = helpers::load_manifest(&layout)?;
    maestria_daemon::reconcile_retrieval_generations(&layout, &mut state, &manifest)
        .context("reconcile retrieval generations before search")?;
    let (runtime, plan, outcome) = execute_search(&layout, &state, &manifest, query, limit).await?;
    persist_search_knowledge(&runtime, &mut state, task_id, &plan, &outcome)?;
    print_search_outcome(&state, &outcome);
    Ok(())
}

pub(crate) async fn execute_search(
    layout: &InstanceLayout,
    state: &maestria_domain::KernelState,
    manifest: &InstanceManifest,
    query: String,
    limit: usize,
) -> Result<(
    std::sync::Arc<maestria_daemon::SearchRuntime>,
    SearchPlan,
    SearchOutcome,
)> {
    let runtime = maestria_daemon::prepare_search_runtime(
        layout,
        state,
        manifest,
        maestria_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
    )?;
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
