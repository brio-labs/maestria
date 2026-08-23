use std::sync::Arc;

use anyhow::{Result, anyhow};
use maestria_domain::{
    EvidenceCandidate, EvidenceSpan, RetrievalLaneScore, RetrievalRawRank, RetrievalScoreKind,
    RetrievalScoreScale, SearchOutcome,
};
use maestria_storage_sqlite::SqliteStore;

use super::super::protocol::{
    CoverageResponse, RetrievalLaneStatus, RetrievalPromotionRecordWire, RetrievalPromotionRecords,
    RetrievalStatusResponse, SearchEvidenceResponse, SearchRawRankResponse, SearchResponse,
    SearchScoreResponse, SearchScoreScaleResponse,
};
use super::super::server::ApiContext;

pub(super) async fn search_with_retry(
    context: &ApiContext,
    query: String,
    limit: usize,
) -> Result<SearchResponse> {
    super::support::run_database_retry_async("search query", || {
        search(context, query.clone(), limit)
    })
    .await
}

async fn search(context: &ApiContext, query: String, limit: usize) -> Result<SearchResponse> {
    // Reuse the daemon runtime's own search executor when the API server is
    // running inside an instance lifecycle (R28: one owner of the retrieval
    // graph). The standalone read-only assembly remains only for servers
    // without a live runtime.
    let (plan, outcome) = match context.runtime.as_ref().and_then(|h| h.search_executor()) {
        Some(executor) => {
            let (plan, outcome) = executor
                .plan_and_search(query, limit)
                .await
                .map_err(|error| anyhow!("search query execution: {error}"))?;
            (plan, outcome)
        }
        None => {
            let runtime = prepare_read_only_search_runtime(context).await?;
            runtime.execute_arc(query, limit).await?
        }
    };
    Ok(search_response(
        plan.original_query().to_string(),
        plan.query_id().value(),
        outcome,
    ))
}

async fn prepare_read_only_search_runtime(
    context: &ApiContext,
) -> Result<Arc<crate::SearchRuntime>> {
    let layout = context.layout.clone();
    let (state, manifest) = tokio::task::spawn_blocking(move || {
        super::support::load_search_generations_and_manifest(&layout)
    })
    .await
    .map_err(|error| anyhow!("load search state task failed: {error}"))??;
    let layout = context.layout.clone();
    tokio::task::spawn_blocking(move || {
        crate::prepare_search_runtime_read_only(
            &layout,
            &state,
            &manifest,
            maestria_governance::RetrievalSecurityPolicy::default()
                .require_read_allowed(true)
                .allow_unscoped_items(true),
        )
    })
    .await
    .map_err(|error| anyhow!("prepare search runtime task failed: {error}"))?
}

/// Assemble the retrieval status read: lane execution states, the promotion
/// records backing them, and instance model configuration.
///
/// Lane states come from the same fail-closed derivations the live runtime
/// applies at construction (`runtime_construction::{hybrid_policy,
/// learned_sparse_policy}`), so the reported states are the policies that
/// actually govern retrieval. The repository-code and visual lanes have no
/// promotion path in runtime construction today and always serve shadow.
/// Any error fails the whole read closed; no partial status is fabricated.
pub(super) async fn retrieval_status(context: &ApiContext) -> Result<RetrievalStatusResponse> {
    let layout = context.layout.clone();
    let (state, manifest) = tokio::task::spawn_blocking(move || {
        super::support::load_search_generations_and_manifest(&layout)
    })
    .await
    .map_err(|error| anyhow!("load retrieval status task failed: {error}"))??;
    let store = SqliteStore::open_read_only(&context.layout.database_path)?;
    let (primary_generation, corpus_snapshot, dense_generation) =
        crate::projection_open::resolve_index_generations(&state)?;
    let hybrid = crate::runtime_construction::hybrid_policy(&store);
    let learned_sparse = crate::runtime_construction::learned_sparse_policy(&store, &manifest);
    let sparse_record = store.load_latest_promotion_record()?;
    let hybrid_record = store.load_latest_hybrid_promotion_record()?;
    let fingerprint = match context
        .runtime
        .as_ref()
        .and_then(|handle| handle.search_executor())
        .and_then(|executor| {
            executor
                .as_any()
                .and_then(|any| any.downcast_ref::<crate::SearchRuntime>())
                .map(|runtime| runtime.fingerprint.clone())
        }) {
        Some(fingerprint) => fingerprint,
        None => maestria_domain::RetrievalModelFingerprint::new(
            "maestria-core:deterministic-v1".to_string(),
        )
        .map_err(|error| anyhow!("invalid fallback model fingerprint: {error}"))?,
    };
    let (hybrid_state, hybrid_served_classes, hybrid_evaluation_id, hybrid_evaluation_date) =
        match &hybrid {
            maestria_retrieval::HybridExecutionPolicy::Shadow => {
                ("Shadow".to_string(), Vec::new(), None, None)
            }
            maestria_retrieval::HybridExecutionPolicy::Active(record) => (
                "Active".to_string(),
                record
                    .served_classes()
                    .iter()
                    .map(|class| format!("{class:?}"))
                    .collect(),
                Some(record.evaluation_id().to_string()),
                Some(record.evaluation_date().to_string()),
            ),
        };
    let lanes = RetrievalLaneStatus {
        hybrid_state,
        hybrid_served_classes,
        hybrid_evaluation_id,
        hybrid_evaluation_date,
        hybrid_report_hash: hybrid_record
            .as_ref()
            .map(|record| record.report_hash.clone()),
        learned_sparse_state: format!("{learned_sparse:?}"),
        learned_sparse_model: manifest
            .sparse
            .as_ref()
            .filter(|profile| profile.enabled)
            .map(|profile| profile.model.clone()),
        dense_enabled: dense_generation.is_some(),
        dense_model: manifest
            .embeddings
            .as_ref()
            .filter(|config| config.enabled)
            .map(|config| config.model.clone()),
        repository_code_state: format!(
            "{:?}",
            maestria_retrieval::RepositoryExecutionPolicy::Shadow
        ),
        visual_state: format!("{:?}", maestria_retrieval::VisualExecutionPolicy::Shadow),
    };
    Ok(RetrievalStatusResponse {
        index_generation: primary_generation.value(),
        corpus_snapshot: corpus_snapshot.value(),
        fingerprint: fingerprint.as_str().to_string(),
        lanes,
        promotion_records: RetrievalPromotionRecords {
            learned_sparse: sparse_record
                .as_ref()
                .map(|record| RetrievalPromotionRecordWire {
                    evaluation_id: record.evaluation_id.clone(),
                    corpus_id: record.corpus_id.clone(),
                    evaluation_date: record.evaluation_date.clone(),
                    report_hash: record.report_hash.clone(),
                    created_at: record.created_at.clone(),
                }),
            hybrid: hybrid_record
                .as_ref()
                .map(|record| RetrievalPromotionRecordWire {
                    evaluation_id: record.evaluation_id.clone(),
                    corpus_id: record.corpus_id.clone(),
                    evaluation_date: record.evaluation_date.clone(),
                    report_hash: record.report_hash.clone(),
                    created_at: record.created_at.clone(),
                }),
        },
    })
}

pub(super) fn search_response(
    query: String,
    query_id: u64,
    outcome: SearchOutcome,
) -> SearchResponse {
    SearchResponse {
        query,
        query_id,
        trace_id: outcome.trace.value(),
        status: format!("{:?}", outcome.status),
        fingerprint: outcome.fingerprint.as_str().to_string(),
        index_generation: outcome.index_generation.value(),
        evidence: outcome.evidence.iter().map(search_evidence).collect(),
        coverage: CoverageResponse {
            percent_covered: outcome.coverage.percent_covered(),
            gaps: outcome.coverage.gaps_identified().to_vec(),
            distinct_sources: outcome.coverage.distinct_sources(),
            distinct_documents: outcome.coverage.distinct_documents(),
            distinct_sections: outcome.coverage.distinct_sections(),
        },
        conflict_count: outcome.conflicts.len(),
    }
}

fn search_evidence(candidate: &EvidenceCandidate) -> SearchEvidenceResponse {
    SearchEvidenceResponse {
        evidence_id: candidate.evidence_id().value(),
        artifact_version: candidate.artifact_version().value(),
        source: format_source_span(candidate.source_span()),
        range_start: candidate.source_span().range().start(),
        range_end: candidate.source_span().range().end(),
        score_schema_version: candidate.scores().schema_version(),
        scores: candidate
            .scores()
            .lanes()
            .iter()
            .map(search_score)
            .collect(),
        trust: format!("{:?}", candidate.trust()),
        freshness: format!("{:?}", candidate.freshness()),
    }
}

fn search_score(score: &RetrievalLaneScore) -> SearchScoreResponse {
    SearchScoreResponse {
        score_kind: score_kind_name(&score.score_kind),
        raw_score: score.raw_score,
        raw_rank: match &score.raw_rank {
            RetrievalRawRank::Ranked { rank } => SearchRawRankResponse::Ranked { rank: *rank },
            RetrievalRawRank::Unavailable { reason } => SearchRawRankResponse::Unavailable {
                reason: reason.clone(),
            },
        },
        scale: match &score.scale {
            RetrievalScoreScale::Binary => SearchScoreScaleResponse::Binary,
            RetrievalScoreScale::Unbounded {
                name,
                higher_is_better,
            } => SearchScoreScaleResponse::Unbounded {
                name: name.clone(),
                higher_is_better: *higher_is_better,
            },
            RetrievalScoreScale::FixedPoint {
                name,
                denominator,
                minimum,
                maximum,
                higher_is_better,
            } => SearchScoreScaleResponse::FixedPoint {
                name: name.clone(),
                denominator: *denominator,
                minimum: *minimum,
                maximum: *maximum,
                higher_is_better: *higher_is_better,
            },
            RetrievalScoreScale::RankDerived {
                name,
                higher_is_better,
            } => SearchScoreScaleResponse::RankDerived {
                name: name.clone(),
                higher_is_better: *higher_is_better,
            },
        },
        representation: score.representation.0.clone(),
        fingerprint: score.fingerprint.identity.as_str().to_string(),
        fingerprint_components: score.fingerprint.components.clone(),
    }
}

fn score_kind_name(kind: &RetrievalScoreKind) -> String {
    match kind {
        RetrievalScoreKind::Exact => "exact".to_string(),
        RetrievalScoreKind::LexicalBm25 => "lexical_bm25".to_string(),
        RetrievalScoreKind::DenseSimilarity => "dense_similarity".to_string(),
        RetrievalScoreKind::LearnedSparse => "learned_sparse".to_string(),
        RetrievalScoreKind::LateInteraction => "late_interaction".to_string(),
        RetrievalScoreKind::Graph => "graph".to_string(),
        RetrievalScoreKind::SpecializedRetrieval { route } => {
            format!("specialized_retrieval:{route}")
        }
    }
}

fn format_source_span(span: &EvidenceSpan) -> String {
    match span.location() {
        maestria_domain::SourceLocation::File {
            path,
            start_line,
            end_line,
        } => format!("{path}:{start_line}-{end_line}"),
        maestria_domain::SourceLocation::Page {
            page_start,
            page_end,
        } => format!("pages {page_start}-{page_end}"),
        maestria_domain::SourceLocation::Region {
            page,
            x,
            y,
            width,
            height,
        } => format!("page {page} region {x},{y} {width}x{height}"),
        maestria_domain::SourceLocation::Symbol {
            path,
            qualified_name,
        } => format!("{path}::{qualified_name}"),
    }
}
