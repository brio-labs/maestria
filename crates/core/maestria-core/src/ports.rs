use crate::error::CoreResult;
use crate::retrieval::{open_chunk_evidence, open_evidence};
use crate::types::{
    OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput, SearchInput, SearchOutput,
};
use maestria_domain::{
    ArtifactVersionId, ContentRange, EvidenceCandidate, EvidenceSpan, FreshnessStatus,
    RetrievalReason, RetrievalScoreSet, SearchOutcome, SearchStatus, SearchStopReason, SearchTrace,
    SearchTraceExpansion, SearchTraceFilter, SourceLocation, SourceSpan, TrustLabel,
};

use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EventLog, FullTextIndex, Parser,
};

pub struct CorePorts<'a> {
    pub artifacts: &'a dyn ArtifactRepository,
    pub chunks: &'a dyn ChunkRepository,
    pub cards: &'a dyn CardRepository,
    pub evidence: &'a dyn maestria_ports::EvidenceRepository,
    pub events: &'a dyn EventLog,
    pub parser: &'a dyn Parser,
    pub search_index: &'a dyn FullTextIndex,
    pub blobs: &'a dyn BlobStore,
    pub vector_index: Option<&'a dyn maestria_ports::VectorIndex>,
    pub graph_index: Option<&'a dyn maestria_ports::GraphIndex>,
}

pub struct CoreServices<'a> {
    ports: CorePorts<'a>,
    graph_config: Option<crate::types::GraphConfig>,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    hybrid_policy: crate::types::HybridExecutionPolicy,
}

pub(super) fn evidence_candidate_from_hit(
    hit: crate::types::SourceGroundedSearchHit,
    reason: &RetrievalReason,
    semantic: bool,
) -> Option<EvidenceCandidate> {
    let (location, range) = match &hit.evidence.kind {
        maestria_domain::EvidenceKind::FileSpan { path, range, .. } => {
            let (start_line, end_line) = match hit.chunk.source_span {
                SourceSpan::TextSpan {
                    start_line,
                    end_line,
                } => (start_line as u32, end_line as u32),
                SourceSpan::PdfSpan { .. } => return None,
            };
            (
                SourceLocation::File {
                    path: path.clone(),
                    start_line,
                    end_line,
                },
                *range,
            )
        }
        maestria_domain::EvidenceKind::PdfSpan {
            page_start,
            page_end,
            ..
        } => (
            SourceLocation::Page {
                page_start: *page_start,
                page_end: *page_end,
            },
            ContentRange { start: 0, end: 1 },
        ),
        _ => return None,
    };
    let source_span = EvidenceSpan::new(Some(hit.chunk.node_id), location, range).ok()?;
    let security = hit.artifact.security.taint_from(&hit.evidence.security);
    let trust = match (&security.trust_zone, &security.integrity) {
        (
            maestria_domain::TrustZone::System | maestria_domain::TrustZone::Verified,
            maestria_domain::IntegrityState::Verified,
        ) => TrustLabel::Verified,
        _ => TrustLabel::Unverified,
    };
    Some(EvidenceCandidate {
        evidence_id: hit.evidence.id,
        // Legacy artifacts have no separate version identity; preserve the
        // artifact identity rather than inventing a hash-based version.
        artifact_version: ArtifactVersionId::new(hit.artifact.id.value()),
        source_span,
        scores: RetrievalScoreSet {
            bm25: if semantic { 0 } else { hit.score },
            semantic_similarity: if semantic { hit.score } else { 0 },
        },
        trust,
        freshness: FreshnessStatus::Unknown,
        duplicate_cluster: None,
        reasons: vec![reason.clone()],
        coverage_keys: Vec::new(),
    })
}
struct SearchTraceOptions<'a> {
    expansion_enabled: bool,
    graph_enabled: bool,
    policy: &'a maestria_governance::RetrievalSecurityPolicy,
    lane_reports: &'a [crate::types::RetrievalLaneReport],
    diversity: Option<maestria_domain::SearchTraceDiversity>,
}

fn build_trace_filters(
    plan: &maestria_domain::SearchPlan,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> Vec<SearchTraceFilter> {
    let mut filters = vec![
        SearchTraceFilter::Quarantine,
        SearchTraceFilter::PromptInjection,
    ];
    if matches!(plan.scope, maestria_domain::CorpusScope::Restricted(_))
        || policy.required_scope_id.is_some()
    {
        filters.push(SearchTraceFilter::Scope);
    }
    if policy.require_read_allowed {
        filters.push(SearchTraceFilter::Acl);
    }
    if policy.require_trust_zone.is_some() {
        filters.push(SearchTraceFilter::Trust);
    }
    if policy.max_sensitivity.is_some() {
        filters.push(SearchTraceFilter::Sensitivity);
    }
    if !matches!(plan.freshness, maestria_domain::FreshnessRequirement::Any) {
        filters.push(SearchTraceFilter::Freshness);
    }
    filters
}

fn build_search_trace(
    plan: &maestria_domain::SearchPlan,
    evidence: &[EvidenceCandidate],
    status: &SearchStatus,
    gaps: &[String],
    options: SearchTraceOptions<'_>,
) -> SearchTrace {
    let SearchTraceOptions {
        expansion_enabled,
        graph_enabled,
        policy,
        lane_reports,
        diversity,
    } = options;
    let stop_reason = match status {
        SearchStatus::NoEvidenceFound => SearchStopReason::NoEvidence,
        SearchStatus::DeniedByPolicy | SearchStatus::QuarantinedForReview => {
            SearchStopReason::PolicyDenied
        }
        SearchStatus::Abstained => SearchStopReason::Abstained,
        SearchStatus::EvidenceIncomplete
        | SearchStatus::StaleEvidenceOnly
        | SearchStatus::SourcesConflict => SearchStopReason::RequirementsUnmet,
        _ => diversity.as_ref().map_or_else(
            || {
                if evidence.len() >= plan.stop_conditions.max_results as usize {
                    SearchStopReason::ResultsLimit
                } else {
                    SearchStopReason::EvidenceComplete
                }
            },
            |trace| trace.stop_reason.clone(),
        ),
    };
    let expansions = expansion_enabled
        .then_some(SearchTraceExpansion {
            strategy: if graph_enabled {
                "hierarchy+graph".to_string()
            } else {
                "hierarchy".to_string()
            },
            added_candidates: None,
        })
        .into_iter()
        .collect();
    let filters = build_trace_filters(plan, policy);
    let lanes = lane_reports
        .iter()
        .map(|report| maestria_domain::SearchTraceLane {
            retriever_id: report.retriever_id.clone(),
            status: match &report.status {
                crate::types::RetrievalLaneStatus::Succeeded => {
                    maestria_domain::SearchLaneStatus::Succeeded
                }
                crate::types::RetrievalLaneStatus::Empty => {
                    maestria_domain::SearchLaneStatus::Empty
                }
                crate::types::RetrievalLaneStatus::Failed { error } => {
                    maestria_domain::SearchLaneStatus::Failed {
                        error: error.clone(),
                    }
                }
            },
            candidates: report.candidates.clone(),
        })
        .collect::<Vec<_>>();
    let mut rewrite_session = maestria_retrieval::rewrite::QueryRewriteSession::with_limits(
        &plan.original_query,
        plan.budgets.max_tokens() as usize,
        plan.budgets.max_latency_ms(),
        plan.budgets.max_queries(),
    );
    rewrite_session.expand_deterministic();
    let mut trace = SearchTrace::from_plan(
        plan,
        lanes.iter().map(|lane| lane.retriever_id.clone()).collect(),
        evidence,
        filters,
        Some("rrf-fixed-k60".to_string()),
        expansions,
        stop_reason,
    )
    .with_lanes(lanes)
    .with_gaps_and_conflicts(gaps.to_vec(), Vec::new())
    .with_policy_fingerprint(format!(
        "trust={:?};sensitivity={:?};read_allowed={};scope={:?};unscoped={}",
        policy.require_trust_zone,
        policy.max_sensitivity,
        policy.require_read_allowed,
        policy.required_scope_id,
        policy.allow_unscoped_items,
    ));
    trace.rewrites = rewrite_session.trace_records();
    trace.diversity = diversity;
    trace
}

fn finish_search_knowledge(
    plan: &maestria_domain::SearchPlan,
    evidence: Vec<EvidenceCandidate>,
    status: SearchStatus,
    coverage: maestria_domain::EvidenceCoverage,
    options: SearchTraceOptions<'_>,
) -> CoreResult<SearchOutcome> {
    let trace_data =
        build_search_trace(plan, &evidence, &status, &coverage.gaps_identified, options);
    let outcome = SearchOutcome {
        trace: trace_data.deterministic_id(),
        trace_data: Some(Box::new(trace_data)),
        fingerprint: plan.fingerprint.clone(),
        index_generation: plan.index_generation,
        status,
        coverage,
        conflicts: Vec::new(),
        evidence,
    };
    outcome
        .verify_compatibility(plan)
        .map_err(|error| crate::error::CoreError::InvalidInput {
            message: error.to_string(),
        })?;
    Ok(outcome)
}

impl<'a> CoreServices<'a> {
    pub fn new(ports: CorePorts<'a>) -> Self {
        Self {
            ports,
            graph_config: Some(crate::types::GraphConfig::default()),
            retrieval_policy: maestria_governance::RetrievalSecurityPolicy::default(),
            hybrid_policy: crate::types::HybridExecutionPolicy::default(),
        }
    }

    pub fn with_retrieval_policy(
        mut self,
        policy: maestria_governance::RetrievalSecurityPolicy,
    ) -> Self {
        self.retrieval_policy = policy;
        self
    }

    pub fn with_hybrid_policy(mut self, policy: crate::types::HybridExecutionPolicy) -> Self {
        self.hybrid_policy = policy;
        self
    }
    pub fn with_graph_config(mut self, config: crate::types::GraphConfig) -> Self {
        self.graph_config = Some(config);
        self
    }

    pub fn search(&self, input: SearchInput) -> CoreResult<SearchOutput> {
        crate::retrieval::search(
            &self.ports,
            input,
            None,
            self.graph_config.clone(),
            &self.retrieval_policy,
            self.hybrid_policy.clone(),
        )
    }
    pub fn search_knowledge(&self, plan: maestria_domain::SearchPlan) -> CoreResult<SearchOutcome> {
        let output = crate::retrieval::search_with_plan(
            &self.ports,
            plan.clone(),
            None,
            self.graph_config.clone(),
            &self.retrieval_policy,
            self.hybrid_policy.clone(),
        )?;
        let reason = RetrievalReason::ExactMatch;
        let lane_reports = output.lane_reports.clone();
        let evidence = output
            .pack
            .chunks
            .into_iter()
            .filter_map(|hit| evidence_candidate_from_hit(hit, &reason, false))
            .collect::<Vec<_>>();
        let ranked = evidence
            .into_iter()
            .enumerate()
            .map(|(rank, candidate)| maestria_retrieval::types::RankedCandidate { candidate, rank })
            .collect::<Vec<_>>();
        let diversity = maestria_retrieval::diversity::select_candidates(&ranked, &plan);
        let evidence = diversity
            .candidates
            .iter()
            .map(|candidate| candidate.candidate.clone())
            .collect::<Vec<_>>();
        let enough_corroboration =
            evidence.len() >= usize::from(plan.evidence_requirements.minimum_corroboration);
        let primary_sources_ok = !plan.evidence_requirements.require_primary_sources
            || evidence
                .iter()
                .all(|candidate| candidate.trust == TrustLabel::Verified);
        let diversity_status = diversity.status.clone();
        let diversity_trace = diversity.trace.clone();
        let mut coverage = diversity.coverage.clone();
        let mut status = if evidence.is_empty() {
            SearchStatus::NoEvidenceFound
        } else if enough_corroboration && primary_sources_ok {
            SearchStatus::Answerable
        } else {
            SearchStatus::EvidenceIncomplete
        };
        if matches!(
            diversity_status,
            SearchStatus::NoEvidenceFound
                | SearchStatus::EvidenceIncomplete
                | SearchStatus::StaleEvidenceOnly
        ) {
            status = diversity_status;
        } else if status == SearchStatus::Answerable
            && diversity_status == SearchStatus::AnswerableWithWarnings
        {
            status = SearchStatus::AnswerableWithWarnings;
        }
        if !enough_corroboration {
            coverage
                .gaps_identified
                .push("minimum corroboration not met".to_string());
        }
        if !primary_sources_ok {
            coverage
                .gaps_identified
                .push("primary-source requirement not met".to_string());
        }
        coverage.percent_covered = if evidence.is_empty() {
            0
        } else if coverage.gaps_identified.is_empty() {
            100
        } else {
            coverage.percent_covered.min(99)
        };
        finish_search_knowledge(
            &plan,
            evidence,
            status,
            coverage,
            SearchTraceOptions {
                expansion_enabled: self.graph_config.is_some(),
                graph_enabled: self.ports.graph_index.is_some(),
                policy: &self.retrieval_policy,
                lane_reports: &lane_reports,
                diversity: Some(diversity_trace),
            },
        )
    }

    pub fn search_with_vector(
        &self,
        input: SearchInput,
        vector_query: maestria_ports::VectorSearchQuery,
    ) -> CoreResult<SearchOutput> {
        crate::retrieval::search(
            &self.ports,
            input,
            Some(vector_query),
            self.graph_config.clone(),
            &self.retrieval_policy,
            self.hybrid_policy.clone(),
        )
    }
    pub fn open_evidence(&self, input: OpenEvidenceInput) -> CoreResult<OpenEvidenceOutput> {
        open_evidence(&self.ports, input, &self.retrieval_policy)
    }

    pub fn open_chunk_evidence(
        &self,
        input: OpenChunkEvidenceInput,
    ) -> CoreResult<OpenEvidenceOutput> {
        open_chunk_evidence(&self.ports, input, &self.retrieval_policy)
    }
}
