use crate::error::{CoreError, CoreResult};
use crate::hierarchy_expansion;
use crate::lane_fusion::{run_cards_lane, run_chunk_lane};
use crate::ports::CorePorts;
use crate::rank_fusion::{
    RankedRetrievalCandidate, RetrievalCandidate, RetrievalLane, fuse, rank_expanded,
};
use crate::types::{EvidencePack, RetrievalMode, SearchInput, SearchOutput};
use maestria_ports::VectorSearchQuery;

fn build_expander<'a>(
    ports: &'a CorePorts<'a>,
    query: String,
    limit: usize,
    graph_config: Option<crate::types::GraphConfig>,
    policy: &'a maestria_governance::RetrievalSecurityPolicy,
) -> impl Fn(
    Vec<RankedRetrievalCandidate>,
    &maestria_domain::SearchPlan,
) -> maestria_retrieval::RetrievalResult<Vec<RankedRetrievalCandidate>>
+ 'a {
    move |candidates, plan| {
        let Some(config) = &graph_config else {
            return Ok(candidates);
        };
        let candidates = candidates
            .into_iter()
            .filter(|ranked| ranked.priority_score >= plan.stop_conditions.min_score_threshold)
            .collect::<Vec<_>>();
        use std::collections::BTreeMap;
        let priorities: BTreeMap<_, _> = candidates
            .iter()
            .map(|ranked| (ranked.candidate.identity(), ranked.priority_score))
            .collect();
        let mut artifact_priorities: BTreeMap<maestria_domain::ArtifactId, u32> = BTreeMap::new();
        for ranked in &candidates {
            if ranked.priority_score < plan.stop_conditions.min_score_threshold {
                continue;
            }
            if let Some(artifact_id) = ranked.candidate.artifact_id() {
                artifact_priorities
                    .entry(artifact_id)
                    .and_modify(|priority| *priority = (*priority).max(ranked.priority_score))
                    .or_insert(ranked.priority_score);
            }
        }
        let mut pack = EvidencePack {
            query: query.clone(),
            cards: Vec::new(),
            chunks: Vec::new(),
            evidence_ids: Vec::new(),
        };
        for ranked in candidates {
            match ranked.candidate {
                RetrievalCandidate::Card(hit) => pack.cards.push(hit),
                RetrievalCandidate::Chunk(hit) => {
                    pack.evidence_ids.push(hit.evidence.id);
                    pack.chunks.push(hit);
                }
                RetrievalCandidate::EvidenceId(id) => pack.evidence_ids.push(id),
            }
        }
        let expanded = hierarchy_expansion::expand(ports, pack, &query, limit, config, policy)
            .map_err(|error| maestria_retrieval::RetrievalError::Internal(error.to_string()))?;
        let expanded = crate::graph_retrieval::expand_graph(ports, expanded, limit, config, policy)
            .map_err(|error| maestria_retrieval::RetrievalError::Internal(error.to_string()))?;
        let mut result = expanded
            .cards
            .into_iter()
            .map(RetrievalCandidate::Card)
            .chain(expanded.chunks.into_iter().map(RetrievalCandidate::Chunk))
            .chain(
                expanded
                    .evidence_ids
                    .into_iter()
                    .map(RetrievalCandidate::EvidenceId),
            )
            .collect::<Vec<_>>();
        let fallback_priority = priorities
            .values()
            .copied()
            .min()
            .map_or(0, |score| score / 2);
        let mut result_priorities = priorities.clone();
        for candidate in &result {
            if result_priorities.contains_key(&candidate.identity()) {
                continue;
            }
            let priority = candidate
                .artifact_id()
                .and_then(|artifact_id| artifact_priorities.get(&artifact_id).copied())
                .map_or(fallback_priority, |score| score / 2);
            result_priorities.insert(candidate.identity(), priority);
        }
        result.sort_by(|left, right| {
            let left_priority = result_priorities
                .get(&left.identity())
                .copied()
                .map_or(0, |score| score);
            let right_priority = result_priorities
                .get(&right.identity())
                .copied()
                .map_or(0, |score| score);
            right_priority
                .cmp(&left_priority)
                .then_with(|| left.identity().cmp(&right.identity()))
        });
        Ok(rank_expanded(
            RetrievalLane::Hierarchy,
            result,
            &result_priorities,
            fallback_priority,
        ))
    }
}

fn check_latency(
    start: std::time::SystemTime,
    excluded: std::time::Duration,
    max_latency_ms: u32,
) -> CoreResult<()> {
    if max_latency_ms > 0 {
        let elapsed = match start.elapsed() {
            Ok(duration) => duration,
            Err(_) => std::time::Duration::ZERO,
        };
        if elapsed.saturating_sub(excluded).as_millis() as u64 > u64::from(max_latency_ms) {
            return Err(CoreError::InvalidInput {
                message: "retrieval latency budget exhausted".to_string(),
            });
        }
    }
    Ok(())
}

fn initial_rewrite_queries(plan: &maestria_domain::SearchPlan) -> Vec<String> {
    let mut session = maestria_retrieval::rewrite::QueryRewriteSession::with_limits(
        &plan.original_query,
        plan.budgets.max_tokens() as usize,
        plan.budgets.max_latency_ms(),
        plan.budgets.max_queries(),
    );
    session.expand_deterministic();
    session
        .records()
        .iter()
        .filter(|record| record.stage == maestria_retrieval::rewrite::StageRole::InitialRetrieval)
        .map(|record| record.query.clone())
        .collect()
}
struct LaneContext<'a> {
    ports: &'a CorePorts<'a>,
    input: &'a SearchInput,
    policy: &'a maestria_governance::RetrievalSecurityPolicy,
    plan: &'a maestria_domain::SearchPlan,
    start: std::time::SystemTime,
}

struct VectorLaneResult {
    pub run: crate::lane_fusion::LaneRun,
    pub ranked: Vec<RankedRetrievalCandidate>,
    pub excluded: std::time::Duration,
}

fn execute_vector_lane<'a>(
    ctx: &LaneContext<'a>,
    vector_query: Option<VectorSearchQuery>,
    active_hybrid: bool,
    mut excluded: std::time::Duration,
) -> CoreResult<Option<VectorLaneResult>> {
    if let Some(vector_query) = vector_query {
        let dense_start = std::time::SystemTime::now();
        let dense_run = run_chunk_lane(
            ctx.ports,
            RetrievalLane::VectorChunks,
            &ctx.input.query,
            ctx.input.limit,
            Some(vector_query),
            ctx.policy,
        );
        let ranked = if active_hybrid {
            dense_run.ranked.clone()
        } else {
            Vec::new()
        };
        if active_hybrid {
            check_latency(ctx.start, excluded, ctx.plan.budgets.max_latency_ms())?;
        } else {
            excluded = match dense_start.elapsed() {
                Ok(duration) => duration,
                Err(_) => std::time::Duration::ZERO,
            };
            check_latency(ctx.start, excluded, ctx.plan.budgets.max_latency_ms())?;
        }
        return Ok(Some(VectorLaneResult {
            run: dense_run,
            ranked,
            excluded,
        }));
    }
    Ok(None)
}

struct LexicalLanesResult {
    pub runs: Vec<crate::lane_fusion::LaneRun>,
    pub ranked: Vec<Vec<RankedRetrievalCandidate>>,
}

fn execute_lexical_lanes<'a>(
    ctx: &LaneContext<'a>,
    rewrite_queries: &[String],
    excluded: std::time::Duration,
) -> CoreResult<LexicalLanesResult> {
    let mut runs = Vec::with_capacity(rewrite_queries.len());
    let mut ranked = Vec::with_capacity(rewrite_queries.len());
    for rewrite_query in rewrite_queries {
        let lane = if rewrite_query.trim().starts_with('"')
            && rewrite_query.trim().ends_with('"')
            && rewrite_query.trim().len() >= 2
        {
            RetrievalLane::ExactChunks
        } else {
            RetrievalLane::LexicalChunks
        };
        let lexical_run = run_chunk_lane(
            ctx.ports,
            lane,
            rewrite_query,
            ctx.input.limit,
            None,
            ctx.policy,
        );
        ranked.push(lexical_run.ranked.clone());
        runs.push(lexical_run);
        check_latency(ctx.start, excluded, ctx.plan.budgets.max_latency_ms())?;
    }
    Ok(LexicalLanesResult { runs, ranked })
}

fn execute_retrieval_lanes<'a>(
    ports: &'a CorePorts<'a>,
    input: &SearchInput,
    vector_query: Option<VectorSearchQuery>,
    policy: &'a maestria_governance::RetrievalSecurityPolicy,
    active_hybrid: bool,
    plan: &maestria_domain::SearchPlan,
    start: std::time::SystemTime,
) -> CoreResult<(
    Vec<crate::types::RetrievalLaneReport>,
    Vec<Vec<RankedRetrievalCandidate>>,
    std::time::Duration,
)> {
    let mut excluded = std::time::Duration::ZERO;
    let max_latency_ms = plan.budgets.max_latency_ms();
    let rewrite_queries = initial_rewrite_queries(plan);
    let mut runs = Vec::with_capacity(rewrite_queries.len() * 2 + 1);
    for rewrite_query in &rewrite_queries {
        runs.push(run_cards_lane(ports, rewrite_query, input.limit, policy));
        check_latency(start, excluded, max_latency_ms)?;
    }
    let mut fusion_ranked = runs
        .iter()
        .map(|run| run.ranked.clone())
        .collect::<Vec<_>>();
    let ctx = LaneContext {
        ports,
        input,
        policy,
        plan,
        start,
    };
    if let Some(vector_res) = execute_vector_lane(&ctx, vector_query, active_hybrid, excluded)? {
        if active_hybrid {
            fusion_ranked.push(vector_res.ranked);
        }
        runs.push(vector_res.run);
        excluded = vector_res.excluded;
    }

    let lexical_res = execute_lexical_lanes(&ctx, &rewrite_queries, excluded)?;
    runs.extend(lexical_res.runs);
    fusion_ranked.extend(lexical_res.ranked);
    let lane_reports = runs.into_iter().map(|run| run.report).collect::<Vec<_>>();
    Ok((lane_reports, fusion_ranked, excluded))
}

fn build_evidence_pack(
    candidates: Vec<RankedRetrievalCandidate>,
    input: &SearchInput,
    plan: &maestria_domain::SearchPlan,
) -> EvidencePack {
    use std::collections::BTreeSet;

    let mut pack = EvidencePack {
        query: input.query.clone(),
        cards: Vec::new(),
        chunks: Vec::new(),
        evidence_ids: Vec::new(),
    };
    let mut cards = BTreeSet::new();
    let mut chunks = BTreeSet::new();
    for ranked in candidates.into_iter().take(input.limit) {
        if ranked.priority_score < plan.stop_conditions.min_score_threshold {
            continue;
        }
        match ranked.candidate {
            RetrievalCandidate::Card(hit) if cards.insert(hit.card.id) => {
                pack.cards.push(hit);
            }
            RetrievalCandidate::Chunk(hit) if chunks.insert(hit.evidence.id) => {
                pack.evidence_ids.push(hit.evidence.id);
                pack.chunks.push(hit);
            }
            RetrievalCandidate::EvidenceId(id) if chunks.insert(id) => {
                pack.evidence_ids.push(id);
            }
            _ => {}
        }
    }
    pack
}

pub(super) fn execute_pipeline<'a>(
    ports: &'a CorePorts<'a>,
    input: &SearchInput,
    vector_query: Option<VectorSearchQuery>,
    graph_config: Option<crate::types::GraphConfig>,
    policy: &'a maestria_governance::RetrievalSecurityPolicy,
    hybrid_policy: crate::types::HybridExecutionPolicy,
    plan: &maestria_domain::SearchPlan,
) -> CoreResult<SearchOutput> {
    let start = std::time::SystemTime::now();
    let active_hybrid = matches!(
        hybrid_policy,
        crate::types::HybridExecutionPolicy::Active(_)
    );
    let (lane_reports, fusion_ranked, excluded) = execute_retrieval_lanes(
        ports,
        input,
        vector_query,
        policy,
        active_hybrid,
        plan,
        start,
    )?;
    let max_latency_ms = plan.budgets.max_latency_ms();
    let mut candidates = fuse(input.limit, fusion_ranked);
    check_latency(start, excluded, max_latency_ms)?;
    if plan
        .stages
        .contains(&maestria_domain::SearchStage::Filtering)
    {
        candidates = build_expander(
            ports,
            input.query.clone(),
            input.limit,
            graph_config,
            policy,
        )(candidates, plan)
        .map_err(|error| CoreError::InvalidInput {
            message: error.to_string(),
        })?;
    }
    check_latency(start, excluded, max_latency_ms)?;
    let pack = build_evidence_pack(candidates, input, plan);
    check_latency(start, excluded, max_latency_ms)?;
    let retrieval_mode = retrieval_mode(&lane_reports, active_hybrid);
    Ok(SearchOutput {
        pack,
        mode: retrieval_mode,
        lane_reports,
    })
}

fn retrieval_mode(
    lane_reports: &[crate::types::RetrievalLaneReport],
    active_hybrid: bool,
) -> RetrievalMode {
    if lane_reports.iter().any(|report| {
        report.retriever_id == "dense_chunks"
            && matches!(report.status, crate::types::RetrievalLaneStatus::Succeeded)
    }) {
        if active_hybrid {
            RetrievalMode::Hybrid
        } else {
            RetrievalMode::HybridShadow
        }
    } else {
        RetrievalMode::LexicalOnly
    }
}
