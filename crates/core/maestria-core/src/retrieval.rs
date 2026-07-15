use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
pub(super) use crate::retrieval_lanes::{
    open_chunk_evidence, open_evidence, verify_source_snapshot,
};
use crate::retrieval_lanes::{search_cards, search_chunks, search_vector_chunks};
use crate::types::{
    EvidencePack, SearchInput, SearchOutput, SourceGroundedCardHit, SourceGroundedSearchHit,
};
use maestria_ports::VectorSearchQuery;

#[derive(Clone)]
enum RetrievalCandidate {
    Card(SourceGroundedCardHit),
    Chunk(SourceGroundedSearchHit),
    EvidenceId(maestria_domain::EvidenceId),
}

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
            max_results: input.limit.saturating_mul(3) as u32,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
        },
        fingerprint: RetrievalModelFingerprint::new(CORE_RETRIEVAL_FINGERPRINT.to_string())
            .map_err(|error| CoreError::InvalidInput {
                message: error.to_string(),
            })?,
    })
}

type Retriever<'a> = Box<
    dyn Fn(
            &maestria_domain::SearchPlan,
        ) -> maestria_retrieval::RetrievalResult<Vec<RetrievalCandidate>>
        + 'a,
>;

fn build_retrievers<'a>(
    ports: &'a CorePorts<'a>,
    query: &str,
    limit: usize,
    vector_query: Option<VectorSearchQuery>,
    policy: &'a maestria_governance::RetrievalSecurityPolicy,
) -> Vec<Retriever<'a>> {
    let mut retrievers: Vec<Retriever<'_>> = Vec::new();
    let card_query = query.to_string();
    retrievers.push(Box::new(move |_| {
        search_cards(ports, &card_query, limit, policy)
            .map(|cards| cards.into_iter().map(RetrievalCandidate::Card).collect())
            .map_err(|error| maestria_retrieval::RetrievalError::Internal(error.to_string()))
    }));
    if let Some(vector_query) = vector_query {
        let vector_query_text = query.to_string();
        retrievers.push(Box::new(move |_| {
            search_vector_chunks(
                ports,
                &vector_query_text,
                limit,
                Some(vector_query.clone()),
                policy,
            )
            .map(|(chunks, _)| chunks.into_iter().map(RetrievalCandidate::Chunk).collect())
            .map_err(|error| maestria_retrieval::RetrievalError::Internal(error.to_string()))
        }));
    }
    let chunk_query = query.to_string();
    retrievers.push(Box::new(move |_| {
        search_chunks(ports, &chunk_query, limit, policy)
            .map(|(chunks, _)| chunks.into_iter().map(RetrievalCandidate::Chunk).collect())
            .map_err(|error| maestria_retrieval::RetrievalError::Internal(error.to_string()))
    }));
    retrievers
}

fn build_fusion(
    limit: usize,
) -> impl Fn(Vec<Vec<RetrievalCandidate>>) -> maestria_retrieval::RetrievalResult<Vec<RetrievalCandidate>>
{
    move |sets| {
        use std::collections::BTreeSet;
        let mut cards = Vec::with_capacity(limit);
        let mut chunks = Vec::with_capacity(limit);
        let mut card_ids = BTreeSet::new();
        let mut evidence_ids = BTreeSet::new();
        for candidate in sets.into_iter().flatten() {
            match candidate {
                RetrievalCandidate::Card(hit) if card_ids.insert(hit.card.id) => cards.push(hit),
                RetrievalCandidate::Chunk(hit) if evidence_ids.insert(hit.evidence.id) => {
                    chunks.push(hit)
                }
                _ => {}
            }
        }
        cards.truncate(limit);
        chunks.truncate(limit);
        Ok(cards
            .into_iter()
            .map(RetrievalCandidate::Card)
            .chain(chunks.into_iter().map(RetrievalCandidate::Chunk))
            .collect())
    }
}

fn build_expander<'a>(
    ports: &'a CorePorts<'a>,
    query: String,
    limit: usize,
    graph_config: Option<crate::types::GraphConfig>,
    policy: &'a maestria_governance::RetrievalSecurityPolicy,
) -> impl Fn(
    Vec<RetrievalCandidate>,
    &maestria_domain::SearchPlan,
) -> maestria_retrieval::RetrievalResult<Vec<RetrievalCandidate>>
+ 'a {
    move |candidates, _| {
        let Some(config) = &graph_config else {
            return Ok(candidates);
        };
        let mut pack = EvidencePack {
            query: query.clone(),
            cards: Vec::new(),
            chunks: Vec::new(),
            evidence_ids: Vec::new(),
        };
        for candidate in candidates {
            match candidate {
                RetrievalCandidate::Card(hit) => pack.cards.push(hit),
                RetrievalCandidate::Chunk(hit) => {
                    pack.evidence_ids.push(hit.evidence.id);
                    pack.chunks.push(hit);
                }
                RetrievalCandidate::EvidenceId(id) => pack.evidence_ids.push(id),
            }
        }
        let expanded = crate::graph_retrieval::expand_graph(ports, pack, limit, config, policy)
            .map_err(|error| maestria_retrieval::RetrievalError::Internal(error.to_string()))?;
        let mut result = expanded
            .cards
            .into_iter()
            .map(RetrievalCandidate::Card)
            .chain(expanded.chunks.into_iter().map(RetrievalCandidate::Chunk))
            .collect::<Vec<_>>();
        result.extend(
            expanded
                .evidence_ids
                .into_iter()
                .map(RetrievalCandidate::EvidenceId),
        );
        Ok(result)
    }
}

fn execute_pipeline<'a>(
    ports: &'a CorePorts<'a>,
    input: &SearchInput,
    vector_query: Option<VectorSearchQuery>,
    graph_config: Option<crate::types::GraphConfig>,
    policy: &'a maestria_governance::RetrievalSecurityPolicy,
    plan: &maestria_domain::SearchPlan,
) -> CoreResult<SearchOutput> {
    use maestria_retrieval::SyncPipeline;

    let query = input.query.clone();
    let min_score_threshold = plan.stop_conditions.min_score_threshold;
    let evaluator = move |candidates: Vec<RetrievalCandidate>, _: &maestria_domain::SearchPlan| {
        use std::collections::BTreeSet;

        let mut pack = EvidencePack {
            query: query.clone(),
            cards: Vec::new(),
            chunks: Vec::new(),
            evidence_ids: Vec::new(),
        };
        let mut cards = BTreeSet::new();
        let mut chunks = BTreeSet::new();
        for candidate in candidates {
            match candidate {
                RetrievalCandidate::Card(hit)
                    if hit.score >= min_score_threshold && cards.insert(hit.card.id) =>
                {
                    pack.cards.push(hit);
                }
                RetrievalCandidate::Chunk(hit)
                    if hit.score >= min_score_threshold && chunks.insert(hit.evidence.id) =>
                {
                    pack.evidence_ids.push(hit.evidence.id);
                    pack.chunks.push(hit);
                }
                RetrievalCandidate::EvidenceId(id) if chunks.insert(id) => {
                    pack.evidence_ids.push(id);
                }
                _ => {}
            }
        }
        Ok(SearchOutput { pack })
    };

    SyncPipeline::new(
        build_retrievers(ports, &input.query, input.limit, vector_query, policy),
        evaluator,
    )
    .with_fusion(build_fusion(input.limit))
    .with_reranker(|candidates, _| Ok(candidates))
    .with_expander(build_expander(
        ports,
        input.query.clone(),
        input.limit,
        graph_config,
        policy,
    ))
    .run(plan)
    .map_err(|error| CoreError::InvalidInput {
        message: error.to_string(),
    })
}
