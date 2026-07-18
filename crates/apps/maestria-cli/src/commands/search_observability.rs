#[cfg(test)]
use super::search_render::trace_from_outcome;
use super::search_render::{render_trace, trace};
use crate::{commands::search, helpers};

use anyhow::{Context, Result, anyhow};
use maestria_domain::{
    DomainEvent, DomainInput, EvidencePackReproducibilityRecord, SearchKnowledgeCompleted,
    SearchOutcome, SearchPlan, SearchTraceId,
};
use maestria_storage_sqlite::SqliteStore;
use std::{path::PathBuf, time::Duration};

pub(super) struct DurableTrace {
    pub(super) id: SearchTraceId,
    pub(super) plan: SearchPlan,
    pub(super) outcome: SearchOutcome,
}

pub async fn run_search_explain(
    instance_dir: PathBuf,
    task_id: Option<u64>,
    query: String,
    limit: usize,
) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let write_lock = maestria_daemon::try_acquire_instance_write_lock(&layout)?;
    let mut state = helpers::load_kernel_state_with_retry(
        &layout,
        Duration::from_secs(2),
        "load kernel state for search explain",
    )?;
    let task_id = search::validate_task_id(&state, task_id)?;
    let manifest = helpers::load_manifest(&layout)?;
    if write_lock.is_some() {
        maestria_daemon::reconcile_retrieval_generations(&layout, &mut state, &manifest)
            .context("reconcile retrieval generations before explain")?;
    }
    let (runtime, plan, outcome) = search::execute_search(
        &layout,
        &state,
        &manifest,
        query,
        limit,
        write_lock.is_some(),
    )
    .await?;
    if write_lock.is_some() {
        persist_search_trace(&runtime, &mut state, task_id, &plan, &outcome)?;
    }
    render_trace(&plan, &outcome)
}

fn persist_search_trace(
    runtime: &maestria_daemon::SearchRuntime,
    state: &mut maestria_domain::KernelState,
    task_id: Option<maestria_domain::TaskId>,
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

pub fn run_search_trace(instance_dir: PathBuf, trace_id: u64) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let store = SqliteStore::open(&layout.database_path)?;
    let durable = load_trace(&store, SearchTraceId::new(trace_id))?;
    render_trace(&durable.plan, &durable.outcome)
}

pub fn run_search_compare(
    instance_dir: PathBuf,
    experiment_a: u64,
    experiment_b: u64,
) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let store = SqliteStore::open(&layout.database_path)?;
    let left = load_trace(&store, SearchTraceId::new(experiment_a))?;
    let right = load_trace(&store, SearchTraceId::new(experiment_b))?;
    render_trace(&left.plan, &left.outcome)?;
    render_trace(&right.plan, &right.outcome)?;
    println!("experiment_a_trace={}", left.id);
    println!("experiment_b_trace={}", right.id);
    println!(
        "query_equal={}",
        left.plan.original_query == right.plan.original_query
    );
    println!("intent_a={:?}", left.plan.intent);
    println!("intent_b={:?}", right.plan.intent);
    println!("snapshot_a={}", left.plan.corpus_snapshot);
    println!("snapshot_b={}", right.plan.corpus_snapshot);
    println!("generation_a={}", left.plan.index_generation);
    println!("generation_b={}", right.plan.index_generation);
    println!("fingerprint_a={}", left.plan.fingerprint.as_str());
    println!("fingerprint_b={}", right.plan.fingerprint.as_str());
    println!("status_a={:?}", left.outcome.status);
    println!("status_b={:?}", right.outcome.status);
    println!("coverage_a={}%", left.outcome.coverage.percent_covered);
    println!("coverage_b={}%", right.outcome.coverage.percent_covered);
    println!("evidence_count_a={}", left.outcome.evidence.len());
    println!("evidence_count_b={}", right.outcome.evidence.len());
    println!("stop_reason_a={:?}", trace(&left)?.stop_reason);
    println!("stop_reason_b={:?}", trace(&right)?.stop_reason);
    Ok(())
}

fn load_trace(store: &SqliteStore, id: SearchTraceId) -> Result<DurableTrace> {
    let events = super::load_events_from_store(store)?;
    for event in events.iter().rev() {
        if let DomainEvent::SearchKnowledgeCompleted { plan, outcome, .. } = &event.event
            && outcome.trace == id
        {
            let plan = plan.as_deref().ok_or_else(|| {
                anyhow!("trace {id} is non-reproducible: durable search plan is missing")
            })?;
            let trace = outcome.trace_data.as_deref().ok_or_else(|| {
                anyhow!("trace {id} is non-reproducible: trace payload is missing")
            })?;
            outcome
                .verify_compatibility(plan)
                .map_err(|error| anyhow!("trace {id} is non-reproducible: {error}"))?;
            if trace.deterministic_id() != id || !trace.matches_plan(plan) {
                return Err(anyhow!(
                    "trace {id} is non-reproducible: identity does not match its plan"
                ));
            }
            return Ok(DurableTrace {
                id,
                plan: plan.clone(),
                outcome: outcome.clone(),
            });
        }
    }
    for event in events.into_iter().rev() {
        if let DomainEvent::SearchExecuted {
            pack_metadata: Some(metadata),
            ..
        } = event.event
            && metadata.search_trace == Some(id)
        {
            return match metadata.reproducibility {
                EvidencePackReproducibilityRecord::LiveNonReproducible { reason } => {
                    Err(anyhow!("trace {id} is non-reproducible: {reason}"))
                }
                EvidencePackReproducibilityRecord::Frozen(_) => Err(anyhow!(
                    "trace {id} is unavailable: the frozen pack has no durable trace payload"
                )),
            };
        }
    }
    Err(anyhow!("trace {id} was not found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{
        EvidenceCoverage, IndexGenerationId, RetrievalModelFingerprint, SearchStatus,
    };
    use std::error::Error;

    fn empty_outcome() -> Result<SearchOutcome, Box<dyn Error>> {
        Ok(SearchOutcome {
            trace: SearchTraceId::new(7),
            trace_data: None,
            fingerprint: RetrievalModelFingerprint::new("test:fingerprint".to_string())?,
            index_generation: IndexGenerationId::new(1),
            status: SearchStatus::NoEvidenceFound,
            evidence: Vec::new(),
            coverage: EvidenceCoverage {
                percent_covered: 0,
                gaps_identified: Vec::new(),
                required_claims: Vec::new(),
                required_subquestions: Vec::new(),
                distinct_sources: 0,
                distinct_documents: 0,
                distinct_sections: 0,
                candidate_coverage_keys: Vec::new(),
            },
            conflicts: Vec::new(),
        })
    }

    #[test]
    fn missing_trace_identifier_fails_clearly() -> Result<(), Box<dyn Error>> {
        let store = SqliteStore::in_memory()?;
        let error = match load_trace(&store, SearchTraceId::new(99)) {
            Ok(_) => return Err("missing trace unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert!(error.to_string().contains("trace 99 was not found"));
        Ok(())
    }

    #[test]
    fn trace_without_payload_is_non_reproducible() -> Result<(), Box<dyn Error>> {
        let outcome = empty_outcome()?;
        let error = match trace_from_outcome(&outcome) {
            Ok(_) => return Err("trace without payload unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert!(error.to_string().contains("non-reproducible"));
        Ok(())
    }
}
