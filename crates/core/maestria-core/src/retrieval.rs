use crate::error::{CoreError, CoreResult};
use crate::generation_gate::ensure_generation_is_serveable;
use crate::hierarchy_expansion;
use crate::lane_fusion::{run_cards_lane, run_chunk_lane};
use crate::ports::CorePorts;
use crate::rank_fusion::{
    RankedRetrievalCandidate, RetrievalCandidate, RetrievalLane, fuse, rank_expanded,
};
pub(super) use crate::retrieval_lanes::{
    open_chunk_evidence, open_evidence, verify_source_snapshot,
};
use crate::types::{EvidencePack, RetrievalMode, SearchInput, SearchOutput};
use maestria_ports::VectorSearchQuery;

const CORE_CORPUS_SNAPSHOT: u64 = 1;
const CORE_INDEX_GENERATION: u64 = 1;
const CORE_RETRIEVAL_FINGERPRINT: &str = "maestria-core:deterministic-v1";

pub(super) fn search<'a>(
    ports: &CorePorts<'a>,
    input: SearchInput,
    vector_query: Option<VectorSearchQuery>,
    graph_config: Option<crate::types::GraphConfig>,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<SearchOutput> {
    let plan = build_search_plan(&input)?;
    ensure_generation_is_serveable(ports, plan.index_generation)?;
    execute_pipeline(ports, &input, vector_query, graph_config, policy, &plan)
}

pub(super) fn search_with_plan<'a>(
    ports: &CorePorts<'a>,
    plan: maestria_domain::SearchPlan,
    vector_query: Option<VectorSearchQuery>,
    graph_config: Option<crate::types::GraphConfig>,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<SearchOutput> {
    use maestria_domain::{CorpusScope, IndexGenerationId};

    if plan.fingerprint.as_str() != CORE_RETRIEVAL_FINGERPRINT {
        return Err(CoreError::InvalidInput {
            message: format!(
                "unsupported retrieval fingerprint {}",
                plan.fingerprint.as_str()
            ),
        });
    }
    if plan.index_generation != IndexGenerationId::new(CORE_INDEX_GENERATION) {
        return Err(CoreError::InvalidInput {
            message: format!(
                "unsupported index generation {}",
                plan.index_generation.value()
            ),
        });
    }
    ensure_generation_is_serveable(ports, plan.index_generation)?;
    let mut effective_policy = policy.clone();
    match &plan.scope {
        CorpusScope::Restricted(scopes) => {
            let [scope_id] = scopes.as_slice() else {
                return Err(CoreError::InvalidInput {
                    message: "core retrieval requires exactly one restricted scope".to_string(),
                });
            };
            if let Some(required_scope) = effective_policy.required_scope_id
                && required_scope != *scope_id
            {
                return Err(CoreError::InvalidInput {
                    message: "search plan scope exceeds the configured retrieval policy"
                        .to_string(),
                });
            }
            effective_policy.required_scope_id = Some(*scope_id);
        }
        CorpusScope::Global => {
            // A configured policy may narrow a global plan; it is never widened.
        }
    }

    if plan.corpus_snapshot.value() != CORE_CORPUS_SNAPSHOT
        || plan.modalities.values() != [maestria_domain::Modality::Text]
        || !matches!(
            plan.intent,
            maestria_domain::SearchIntent::ExactLookup
                | maestria_domain::SearchIntent::FactualLocal
        )
        || plan.freshness != maestria_domain::FreshnessRequirement::Any
        || plan.stages != [maestria_domain::SearchStage::InitialRetrieval]
    {
        return Err(CoreError::InvalidInput {
            message: "search plan requests unsupported core retrieval capabilities".to_string(),
        });
    }
    let input = SearchInput {
        query: plan.original_query.clone(),
        limit: plan.stop_conditions.max_results as usize,
    };
    execute_pipeline(
        ports,
        &input,
        vector_query,
        graph_config,
        &effective_policy,
        &plan,
    )
}

fn build_search_plan(input: &SearchInput) -> CoreResult<maestria_domain::SearchPlan> {
    use maestria_domain::{
        CorpusScope, CorpusSnapshotId, EvidenceRequirements, FreshnessRequirement,
        IndexGenerationId, Modality, ModalitySet, QueryId, RetrievalModelFingerprint, SearchBudget,
        SearchIntent, SearchPlan, SearchStage, StopConditions,
    };
    Ok(SearchPlan {
        query_id: QueryId::new(1),
        original_query: input.query.clone(),
        intent: SearchIntent::ExactLookup,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(CORE_CORPUS_SNAPSHOT),
        index_generation: IndexGenerationId::new(CORE_INDEX_GENERATION),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(1000, 30_000).map_err(|error| CoreError::InvalidInput {
            message: error.to_string(),
        })?,
        stop_conditions: StopConditions {
            max_results: u32::try_from(input.limit).map_or(u32::MAX, |value| value),
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: vec![],
            required_subquestions: vec![],
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        },
        fingerprint: RetrievalModelFingerprint::new(CORE_RETRIEVAL_FINGERPRINT.to_string())
            .map_err(|error| CoreError::InvalidInput {
                message: error.to_string(),
            })?,
    })
}

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

fn check_latency(start: std::time::Instant, max_latency_ms: u32) -> CoreResult<()> {
    if max_latency_ms > 0 && start.elapsed().as_millis() as u64 > u64::from(max_latency_ms) {
        Err(CoreError::InvalidInput {
            message: "retrieval latency budget exhausted".to_string(),
        })
    } else {
        Ok(())
    }
}

#[allow(clippy::disallowed_methods)]
fn execute_pipeline<'a>(
    ports: &'a CorePorts<'a>,
    input: &SearchInput,
    vector_query: Option<VectorSearchQuery>,
    graph_config: Option<crate::types::GraphConfig>,
    policy: &'a maestria_governance::RetrievalSecurityPolicy,
    plan: &maestria_domain::SearchPlan,
) -> CoreResult<SearchOutput> {
    use std::collections::BTreeSet;
    let start = std::time::Instant::now();
    let max_latency_ms = plan.budgets.max_latency_ms();

    let mut runs = vec![run_cards_lane(ports, &input.query, input.limit, policy)];
    check_latency(start, max_latency_ms)?;
    if vector_query.is_some() {
        runs.push(run_chunk_lane(
            ports,
            RetrievalLane::VectorChunks,
            &input.query,
            input.limit,
            vector_query.clone(),
            policy,
        ));
    }
    check_latency(start, max_latency_ms)?;
    runs.push(run_chunk_lane(
        ports,
        if input.query.trim().starts_with('"')
            && input.query.trim().ends_with('"')
            && input.query.trim().len() >= 2
        {
            RetrievalLane::ExactChunks
        } else {
            RetrievalLane::LexicalChunks
        },
        &input.query,
        input.limit,
        None,
        policy,
    ));
    check_latency(start, max_latency_ms)?;
    let lane_reports = runs
        .iter()
        .map(|run| run.report.clone())
        .collect::<Vec<_>>();
    let mut candidates = fuse(
        input.limit,
        runs.into_iter().map(|run| run.ranked).collect(),
    );
    check_latency(start, max_latency_ms)?;
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
    check_latency(start, max_latency_ms)?;

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
    check_latency(start, max_latency_ms)?;
    let retrieval_mode = if lane_reports.iter().any(|report| {
        report.retriever_id == "dense_chunks"
            && matches!(report.status, crate::types::RetrievalLaneStatus::Succeeded)
    }) {
        RetrievalMode::Hybrid
    } else {
        RetrievalMode::LexicalOnly
    };
    Ok(SearchOutput {
        pack,
        mode: retrieval_mode,
        lane_reports,
    })
}
