#[cfg(test)]
use super::search_render::trace_from_outcome;
use super::search_render::{render_trace, trace};
use crate::{commands::search, helpers};
use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, InstanceService, SearchInput};
use maestria_domain::{
    DomainEvent, DomainInput, EvidencePackReproducibilityRecord, SearchKnowledgeCompleted,
    SearchOutcome, SearchPlan, SearchTraceId,
};
use maestria_governance::AutonomyProfile;
use maestria_parsers::ParserRegistry;
use maestria_ports::{GraphIndex, VectorIndex};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use std::{fs, path::PathBuf};

pub(super) struct DurableTrace {
    pub(super) id: SearchTraceId,
    pub(super) plan: SearchPlan,
    pub(super) outcome: SearchOutcome,
}

pub async fn run_search_explain(instance_dir: PathBuf, query: String, limit: usize) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let _instance_lock = maestria_daemon::acquire_instance_write_lock(&layout).await?;
    let search_layout = layout.clone();
    let result = tokio::task::spawn_blocking(move || explain_search(&search_layout, query, limit))
        .await
        .map_err(|error| anyhow!("search explain worker failed: {error}"))??;
    persist_search_trace(&layout, &result.plan, &result.outcome).await?;
    render_trace(&result.plan, &result.outcome)
}

fn explain_search(
    layout: &maestria_core::InstanceLayout,
    query: String,
    limit: usize,
) -> Result<DurableTrace> {
    let sqlite_store = SqliteStore::open(&layout.database_path)?;
    let blob_store = FsBlobStore::open(&layout.blobs_dir)?;
    let manifest_contents = fs::read_to_string(&layout.manifest_path)?;
    let manifest = InstanceService::parse_manifest(&manifest_contents)?;
    let state = maestria_daemon::load_kernel_state(layout)?;
    let embedding_provider = maestria_daemon::build_embedding_provider(&manifest, &state)?;
    let vector_index = search::open_reconciled_vector_index(
        layout,
        &state,
        &manifest,
        embedding_provider.as_deref(),
    )?;
    let graph_index = search::open_reconciled_graph_index(layout, &state)?;
    let search_index = TantivyFullTextIndex::open(&layout.full_text_index_dir)?;
    search::ensure_search_index(&search_index, &state)?;
    let vector_query = search::build_vector_query(
        vector_index.is_some(),
        embedding_provider.as_deref(),
        &query,
        limit,
    );
    let parser = ParserRegistry::with_defaults();
    let core = CoreServices::new(CorePorts {
        artifacts: &sqlite_store,
        chunks: &sqlite_store,
        cards: &sqlite_store,
        evidence: &sqlite_store,
        events: &sqlite_store,
        parser: &parser,
        search_index: &search_index,
        blobs: &blob_store,
        vector_index: vector_index.as_ref().map(|index| index as &dyn VectorIndex),
        graph_index: graph_index.as_ref().map(|index| index as &dyn GraphIndex),
    });
    let input = SearchInput { query, limit };
    let plan = core.search_plan(input.clone())?;
    let outcome = match vector_query {
        Some(vector_query) => core.explain_search_with_vector(input, vector_query)?,
        None => core.explain_search(input)?,
    };
    let trace = outcome
        .trace_data
        .as_deref()
        .ok_or_else(|| anyhow!("search explain produced no durable trace payload"))?;
    outcome
        .verify_compatibility(&plan)
        .map_err(|error| anyhow!("search explain produced an invalid trace: {error}"))?;
    if trace.deterministic_id() != outcome.trace || !trace.matches_plan(&plan) {
        return Err(anyhow!(
            "search explain produced a non-reproducible trace {}",
            outcome.trace
        ));
    }
    Ok(DurableTrace {
        id: outcome.trace,
        plan,
        outcome,
    })
}

async fn persist_search_trace(
    layout: &maestria_core::InstanceLayout,
    plan: &SearchPlan,
    outcome: &SearchOutcome,
) -> Result<()> {
    let state = maestria_daemon::load_kernel_state(layout)
        .context("load kernel state for search trace persistence")?;
    let event_count_before = state.event_log.len();
    let (runtime, input_tx, input_rx, shutdown_token) =
        maestria_daemon::build_runtime(layout, state, AutonomyProfile::TrustedWorkspace)
            .context("build runtime for search trace persistence")?;
    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));
    let input = DomainInput::SearchKnowledgeCompleted(SearchKnowledgeCompleted {
        task_id: None,
        plan: Some(Box::new(plan.clone())),
        outcome: outcome.clone(),
    });
    let result = async {
        input_tx
            .send(input)
            .await
            .map_err(|error| anyhow!("failed to queue search trace persistence: {error}"))?;
        wait_for_search_trace_persistence(
            layout,
            event_count_before,
            plan,
            outcome,
            std::time::Duration::from_secs(5),
        )
        .await
    }
    .await;
    shutdown_token.cancel();
    let join_result = runtime_task.await;
    result?;
    join_result.context("search trace runtime join failed")?;
    Ok(())
}

async fn wait_for_search_trace_persistence(
    layout: &maestria_core::InstanceLayout,
    event_count_before: usize,
    expected_plan: &SearchPlan,
    expected_outcome: &SearchOutcome,
    timeout_budget: std::time::Duration,
) -> Result<()> {
    tokio::time::timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout) {
                Ok(state) => {
                    if state
                        .event_log
                        .get(event_count_before..)
                        .is_some_and(|events| {
                            events.iter().any(|event| {
                                matches!(
                                    &event.event,
                                    DomainEvent::SearchKnowledgeCompleted {
                                        plan: Some(plan),
                                        outcome,
                                        ..
                                    } if plan.as_ref() == expected_plan && outcome == expected_outcome
                                )
                            })
                        })
                    {
                        return Ok(());
                    }
                }
                Err(error) if helpers::is_db_locked(&error) => {}
                Err(error) => return Err(error),
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| anyhow!("timed out waiting for search trace persistence"))?
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
