use anyhow::{Result, anyhow};
use maestria_domain::{SearchOutcome, SearchPlan, SearchTrace};
use std::collections::BTreeSet;

pub(super) fn trace(durable: &super::search_observability::DurableTrace) -> Result<&SearchTrace> {
    durable.outcome.trace_data.as_deref().ok_or_else(|| {
        anyhow!(
            "trace {} is non-reproducible: trace payload is missing",
            durable.id
        )
    })
}

pub(super) fn render_trace(plan: &SearchPlan, outcome: &SearchOutcome) -> Result<()> {
    let trace = trace_from_outcome(outcome)?;
    println!("trace_id={}", outcome.trace);
    println!("plan_query_id={}", plan.query_id);
    println!("query={}", plan.original_query);
    println!("intent={:?}", plan.intent);
    println!("scope={:?}", plan.scope);
    println!("snapshot={}", plan.corpus_snapshot);
    println!("index_generation={}", plan.index_generation);
    println!("retrieval_fingerprint={}", plan.fingerprint.as_str());
    println!("freshness={:?}", plan.freshness);
    println!("modalities={:?}", plan.modalities);
    println!("stages={:?}", plan.stages);
    println!("budgets={:?}", plan.budgets);
    println!("stop_conditions={:?}", plan.stop_conditions);
    println!("evidence_requirements={:?}", plan.evidence_requirements);
    println!("rewrites={:?}", trace.rewrites);
    println!(
        "retrievers_run={:?}",
        trace
            .lanes
            .iter()
            .map(|lane| &lane.retriever_id)
            .collect::<Vec<_>>()
    );
    let run = trace
        .lanes
        .iter()
        .map(|lane| lane.retriever_id.as_str())
        .collect::<BTreeSet<_>>();
    println!(
        "retrievers_skipped={:?}",
        trace
            .retrievers
            .iter()
            .filter(|retriever| !run.contains(retriever.as_str()))
            .collect::<Vec<_>>()
    );
    println!("raw_candidates={:?}", trace.raw_candidates);
    println!("fusion={:?}", trace.fusion);
    println!("reranked={:?}", trace.rerank);
    println!("filters={:?}", trace.filters);
    println!("expansion={:?}", trace.expansions);
    println!(
        "freshness_trust={:?}",
        trace
            .raw_candidates
            .iter()
            .map(|candidate| (&candidate.freshness, &candidate.trust))
            .collect::<Vec<_>>()
    );
    println!(
        "duplicate_clusters={:?}",
        trace
            .raw_candidates
            .iter()
            .map(|candidate| candidate.duplicate_cluster)
            .collect::<Vec<_>>()
    );
    println!("missing_claims={:?}", trace.missing_evidence);
    println!("conflicts={:?}", trace.conflicts);
    println!("status={:?}", outcome.status);
    println!("coverage={:?}", outcome.coverage);
    println!("stop_reason={:?}", trace.stop_reason);
    Ok(())
}

pub(super) fn trace_from_outcome(outcome: &SearchOutcome) -> Result<&SearchTrace> {
    outcome.trace_data.as_deref().ok_or_else(|| {
        anyhow!(
            "trace {} is non-reproducible: trace payload is missing",
            outcome.trace
        )
    })
}
