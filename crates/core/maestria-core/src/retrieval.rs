use crate::error::{CoreError, CoreResult};
use crate::generation_gate::ensure_generation_is_serveable;
use crate::ports::CorePorts;
pub(super) use crate::retrieval_lanes::{
    open_chunk_evidence, open_evidence, verify_source_snapshot,
};
use crate::types::{EvidencePack, RetrievalMode, SearchInput, SearchOutput};
use maestria_ports::VectorSearchQuery;

#[path = "retrieval_pipeline.rs"]
mod retrieval_pipeline;

const CORE_CORPUS_SNAPSHOT: u64 = 1;
const CORE_INDEX_GENERATION: u64 = 1;
const CORE_RETRIEVAL_FINGERPRINT: &str = "maestria-core:deterministic-v1";

fn empty_search_output(query: String) -> SearchOutput {
    SearchOutput {
        pack: EvidencePack {
            query,
            cards: Vec::new(),
            chunks: Vec::new(),
            evidence_ids: Vec::new(),
        },
        mode: RetrievalMode::LexicalOnly,
        lane_reports: Vec::new(),
    }
}

fn core_search_capabilities(
    semantic_enabled: bool,
    filtering_enabled: bool,
) -> maestria_governance::SearchCapabilities {
    let max_stages = if filtering_enabled { 2 } else { 1 };
    let mut capabilities = maestria_governance::SearchCapabilities::core_defaults(
        maestria_domain::CorpusSnapshotId::new(CORE_CORPUS_SNAPSHOT),
        maestria_domain::IndexGenerationId::new(CORE_INDEX_GENERATION),
        (1_000, 30_000),
    )
    .max_budgets(1_000, 30_000, 8, max_stages, 0);
    if semantic_enabled {
        capabilities = capabilities.with_intent(maestria_domain::SearchIntent::SemanticDiscovery);
    }
    if filtering_enabled {
        capabilities = capabilities.with_stage(maestria_domain::SearchStage::Filtering);
    }
    capabilities
}
fn validate_core_plan(
    ports: &CorePorts<'_>,
    plan: &maestria_domain::SearchPlan,
    policy: &maestria_governance::RetrievalSecurityPolicy,
    semantic_enabled: bool,
    filtering_enabled: bool,
) -> CoreResult<()> {
    maestria_governance::SearchPlanValidator::validate(
        plan,
        &core_search_capabilities(semantic_enabled, filtering_enabled),
        policy,
    )
    .map_err(CoreError::SearchPlan)?;
    if plan.fingerprint.as_str() != CORE_RETRIEVAL_FINGERPRINT {
        return Err(CoreError::InvalidInput {
            message: format!(
                "unsupported retrieval fingerprint {}",
                plan.fingerprint.as_str()
            ),
        });
    }
    ensure_generation_is_serveable(ports, plan.index_generation)?;
    Ok(())
}

pub(super) fn search<'a>(
    ports: &CorePorts<'a>,
    input: SearchInput,
    vector_query: Option<VectorSearchQuery>,
    graph_config: Option<crate::types::GraphConfig>,
    policy: &maestria_governance::RetrievalSecurityPolicy,
    hybrid_policy: crate::types::HybridExecutionPolicy,
) -> CoreResult<SearchOutput> {
    if input.limit == 0 {
        return Ok(empty_search_output(input.query));
    }
    let plan = build_search_plan(&input, graph_config.is_some())?;
    validate_core_plan(
        ports,
        &plan,
        policy,
        vector_query.is_some(),
        graph_config.is_some(),
    )?;
    retrieval_pipeline::execute_pipeline(
        ports,
        &input,
        vector_query,
        graph_config,
        policy,
        hybrid_policy,
        &plan,
    )
}
pub(super) fn search_with_plan<'a>(
    ports: &CorePorts<'a>,
    plan: maestria_domain::SearchPlan,
    vector_query: Option<VectorSearchQuery>,
    graph_config: Option<crate::types::GraphConfig>,
    policy: &maestria_governance::RetrievalSecurityPolicy,
    hybrid_policy: crate::types::HybridExecutionPolicy,
) -> CoreResult<SearchOutput> {
    use maestria_domain::CorpusScope;

    validate_core_plan(
        ports,
        &plan,
        policy,
        vector_query.is_some(),
        graph_config.is_some(),
    )?;

    let mut effective_policy = policy.clone();
    if let CorpusScope::Restricted(scopes) = &plan.scope {
        let [scope_id] = scopes.as_slice() else {
            return Err(CoreError::InvalidInput {
                message: "core retrieval requires exactly one restricted scope".to_string(),
            });
        };
        effective_policy.required_scope_id = Some(*scope_id);
    }
    let input = SearchInput {
        query: plan.original_query.clone(),
        limit: plan.stop_conditions.max_results as usize,
    };
    retrieval_pipeline::execute_pipeline(
        ports,
        &input,
        vector_query,
        graph_config,
        &effective_policy,
        hybrid_policy,
        &plan,
    )
}

fn build_search_plan(
    input: &SearchInput,
    filtering_enabled: bool,
) -> CoreResult<maestria_domain::SearchPlan> {
    use maestria_domain::{
        CorpusScope, CorpusSnapshotId, EvidenceRequirements, FreshnessRequirement,
        IndexGenerationId, Modality, ModalitySet, QueryId, RetrievalModelFingerprint, SearchBudget,
        SearchIntent, SearchPlan, SearchStage, StopConditions,
    };
    Ok(SearchPlan {
        query_id: QueryId::new(1),
        original_query: input.query.clone(),
        intent: SearchIntent::classify(&input.query),
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(CORE_CORPUS_SNAPSHOT),
        index_generation: IndexGenerationId::new(CORE_INDEX_GENERATION),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: if filtering_enabled {
            vec![SearchStage::InitialRetrieval, SearchStage::Filtering]
        } else {
            vec![SearchStage::InitialRetrieval]
        },
        budgets: SearchBudget::with_limits(
            1000,
            30_000,
            8,
            if filtering_enabled { 2 } else { 1 },
            0,
        )
        .map_err(|error| CoreError::InvalidInput {
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
