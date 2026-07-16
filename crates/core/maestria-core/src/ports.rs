use crate::error::CoreResult;
use crate::retrieval::{open_chunk_evidence, open_evidence};
use crate::types::{
    OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput, SearchInput, SearchOutput,
};
use maestria_domain::{
    ArtifactVersionId, ContentHash, ContentRange, EvidenceCandidate, EvidenceCoverage,
    EvidenceSpan, FreshnessStatus, RetrievalReason, RetrievalScoreSet, SearchOutcome, SearchStatus,
    SearchStopReason, SearchTrace, SearchTraceExpansion, SearchTraceFilter, SourceLocation,
    SourceSpan, TrustLabel,
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
}

fn evidence_candidate_from_hit(
    hit: crate::types::SourceGroundedSearchHit,
    reason: &RetrievalReason,
) -> Option<EvidenceCandidate> {
    let content_hash = hit.artifact.content_hash.clone()?;
    if ContentHash::new(content_hash).is_err() {
        return None;
    }
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
            bm25: hit.score,
            semantic_similarity: 0,
        },
        trust,
        freshness: FreshnessStatus::Unknown,
        duplicate_cluster: None,
        reasons: vec![reason.clone()],
    })
}
fn build_search_trace(
    plan: &maestria_domain::SearchPlan,
    evidence: &[EvidenceCandidate],
    status: &SearchStatus,
    gaps: &[String],
    expansion_enabled: bool,
    graph_enabled: bool,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> SearchTrace {
    let chunk_lane = if plan.original_query.trim().starts_with('"') {
        "exact_chunks"
    } else {
        "lexical_chunks"
    };
    let stop_reason = match status {
        SearchStatus::NoEvidenceFound => SearchStopReason::NoEvidence,
        SearchStatus::DeniedByPolicy | SearchStatus::QuarantinedForReview => {
            SearchStopReason::PolicyDenied
        }
        SearchStatus::Abstained => SearchStopReason::Abstained,
        SearchStatus::EvidenceIncomplete
        | SearchStatus::StaleEvidenceOnly
        | SearchStatus::SourcesConflict => SearchStopReason::RequirementsUnmet,
        _ if evidence.len() >= plan.stop_conditions.max_results as usize => {
            SearchStopReason::ResultsLimit
        }
        _ => SearchStopReason::EvidenceComplete,
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
    SearchTrace::from_plan(
        plan,
        vec!["cards".to_string(), chunk_lane.to_string()],
        evidence,
        filters,
        Some("rrf-fixed-k60".to_string()),
        expansions,
        stop_reason,
    )
    .with_gaps_and_conflicts(gaps.to_vec(), Vec::new())
    .with_policy_fingerprint(format!(
        "trust={:?};sensitivity={:?};read_allowed={};scope={:?};unscoped={}",
        policy.require_trust_zone,
        policy.max_sensitivity,
        policy.require_read_allowed,
        policy.required_scope_id,
        policy.allow_unscoped_items,
    ))
}

impl<'a> CoreServices<'a> {
    pub fn new(ports: CorePorts<'a>) -> Self {
        Self {
            ports,
            graph_config: Some(crate::types::GraphConfig::default()),
            retrieval_policy: maestria_governance::RetrievalSecurityPolicy::default(),
        }
    }

    pub fn with_retrieval_policy(
        mut self,
        policy: maestria_governance::RetrievalSecurityPolicy,
    ) -> Self {
        self.retrieval_policy = policy;
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
        )
    }
    pub fn search_knowledge(&self, plan: maestria_domain::SearchPlan) -> CoreResult<SearchOutcome> {
        let output = crate::retrieval::search_with_plan(
            &self.ports,
            plan.clone(),
            None,
            self.graph_config.clone(),
            &self.retrieval_policy,
        )?;
        let reason = match plan.intent {
            maestria_domain::SearchIntent::ExactLookup => RetrievalReason::ExactMatch,
            _ => RetrievalReason::SemanticSimilarity,
        };
        let evidence = output
            .pack
            .chunks
            .into_iter()
            .filter_map(|hit| evidence_candidate_from_hit(hit, &reason))
            .collect::<Vec<_>>();
        let enough_corroboration =
            evidence.len() >= usize::from(plan.evidence_requirements.minimum_corroboration);
        let primary_sources_ok = !plan.evidence_requirements.require_primary_sources
            || evidence
                .iter()
                .all(|candidate| candidate.trust == TrustLabel::Verified);
        let status = if evidence.is_empty() {
            SearchStatus::NoEvidenceFound
        } else if enough_corroboration && primary_sources_ok {
            SearchStatus::Answerable
        } else {
            SearchStatus::EvidenceIncomplete
        };
        let mut gaps_identified = Vec::new();
        if !enough_corroboration {
            gaps_identified.push("minimum corroboration not met".to_string());
        }
        if !primary_sources_ok {
            gaps_identified.push("primary-source requirement not met".to_string());
        }
        let percent_covered = if evidence.is_empty() {
            0
        } else if gaps_identified.is_empty() {
            100
        } else {
            50
        };
        let conflicts = Vec::new();
        let trace_data = build_search_trace(
            &plan,
            &evidence,
            &status,
            &gaps_identified,
            self.graph_config.is_some(),
            self.ports.graph_index.is_some(),
            &self.retrieval_policy,
        );
        let outcome = SearchOutcome {
            trace: trace_data.deterministic_id(),
            trace_data: Some(Box::new(trace_data)),
            fingerprint: plan.fingerprint.clone(),
            index_generation: plan.index_generation,
            status,
            coverage: EvidenceCoverage {
                percent_covered,
                gaps_identified,
            },
            conflicts,
            evidence,
        };
        outcome.verify_compatibility(&plan).map_err(|error| {
            crate::error::CoreError::InvalidInput {
                message: error.to_string(),
            }
        })?;
        Ok(outcome)
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
