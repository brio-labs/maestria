use std::collections::{BTreeMap, BTreeSet};

use maestria_domain::{
    Artifact, ArtifactId, ArtifactVersionId, BlobId, Claim, ClaimId, ClaimStatus, ContentRange,
    CorpusScope, Evidence, EvidenceCandidate, EvidenceCoverage, EvidenceId, EvidenceKind,
    EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus, IndexGenerationId,
    IndexStatus, LogicalTick, Modality, ModalitySet, QueryId, RetrievalModelFingerprint,
    RetrievalReason, RetrievalScoreSet, SearchBudget, SearchIntent, SearchOutcome, SearchPlan,
    SearchStage, SearchStatus, SearchStopReason, SearchTrace, SearchTraceFilter, SecurityMetadata,
    SourceLocation, StopConditions, TrustLabel,
};

use super::{SearchValidationContext, ValidationContext};

pub fn fixture_scores(
    bm25: u32,
    dense: u32,
) -> Result<RetrievalScoreSet, maestria_domain::SearchCompatibilityError> {
    let mut lanes = Vec::new();
    if bm25 != 0 {
        let representation = maestria_domain::RepresentationName::new("lexical_text_v1");
        lanes.push(maestria_domain::RetrievalLaneScore::new(
            maestria_domain::RetrievalScoreKind::LexicalBm25,
            i64::from(bm25),
            maestria_domain::RetrievalRawRank::ranked(1),
            maestria_domain::RetrievalScoreScale::unbounded("fixture_bm25"),
            representation.clone(),
            maestria_domain::RetrievalScoreFingerprint::new(
                maestria_domain::RetrievalModelFingerprint::new(
                    "fixture:lexical-bm25:v1".to_string(),
                )?,
                std::collections::BTreeMap::from([(
                    "representation".to_string(),
                    representation.0,
                )]),
            ),
        ));
    }
    if dense != 0 {
        let representation = maestria_domain::RepresentationName::new("dense_text_v1");
        lanes.push(maestria_domain::RetrievalLaneScore::new(
            maestria_domain::RetrievalScoreKind::DenseSimilarity,
            i64::from(dense),
            maestria_domain::RetrievalRawRank::ranked(1),
            maestria_domain::RetrievalScoreScale::bounded_fixed_point(
                "fixture_dense_micros",
                1_000_000,
                0,
                1_000_000,
            ),
            representation.clone(),
            maestria_domain::RetrievalScoreFingerprint::new(
                maestria_domain::RetrievalModelFingerprint::new(
                    "fixture:dense-similarity:v1".to_string(),
                )?,
                std::collections::BTreeMap::from([(
                    "representation".to_string(),
                    representation.0,
                )]),
            ),
        ));
    }
    RetrievalScoreSet::new(lanes)
}

pub struct SearchFixture {
    pub plan: SearchPlan,
    pub outcome: SearchOutcome,
    pub evidences: BTreeMap<EvidenceId, Evidence>,
    pub artifacts: BTreeMap<ArtifactId, Artifact>,
    pub claims: BTreeMap<maestria_domain::ClaimId, maestria_domain::Claim>,
    pub memory_candidates:
        BTreeMap<maestria_domain::MemoryCandidateId, maestria_domain::MemoryCandidate>,
}

impl SearchFixture {
    pub fn context(&self) -> ValidationContext<'_> {
        ValidationContext {
            task: None,
            artifacts: &self.artifacts,
            claims: &self.claims,
            evidences: &self.evidences,
            memory_candidates: &self.memory_candidates,
            harness_exit_code: None,
            search: Some(SearchValidationContext {
                outcome: &self.outcome,
                plan: Some(&self.plan),
                trace: self.outcome.trace_data.as_deref(),
                evidence_by_id: &self.evidences,
                artifacts_by_id: &self.artifacts,
            }),
        }
    }
}

pub fn plan() -> Result<SearchPlan, Box<dyn std::error::Error>> {
    Ok(SearchPlan {
        query_id: QueryId::new(7),
        original_query: "evidence query".to_string(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: maestria_domain::CorpusSnapshotId::new(8),
        index_generation: IndexGenerationId::new(9),
        freshness: FreshnessRequirement::Realtime,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::with_limits(100, 100, 1, 1, 0)?,
        stop_conditions: StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        },
        fingerprint: RetrievalModelFingerprint::new("validation-fixture-v1".to_string())?,
        original_intent: None,
        route_decision: None,
    })
}

pub fn candidate() -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
    Ok(EvidenceCandidate {
        evidence_id: EvidenceId::new(10),
        artifact_version: ArtifactVersionId::new(12),
        source_span: EvidenceSpan::new(
            None,
            SourceLocation::File {
                path: "notes.md".to_string(),
                start_line: 1,
                end_line: 1,
            },
            ContentRange { start: 1, end: 1 },
        )?,
        scores: fixture_scores(900, 800)?,
        trust: TrustLabel::Unverified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: None,
        reasons: vec![RetrievalReason::ExactMatch],
        coverage_keys: Vec::new(),
    })
}

pub fn evidence() -> Evidence {
    Evidence {
        id: EvidenceId::new(10),
        artifact_id: ArtifactId::new(12),
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "notes.md".to_string(),
            range: ContentRange { start: 1, end: 1 },
            content_hash: "sha256:fixture".to_string(),
            snapshot: Some(BlobId::new(13)),
        },
        excerpt: "evidence excerpt".to_string(),
        observed_at: LogicalTick::new(14),
        security: SecurityMetadata::default(),
    }
}

pub fn fixture() -> Result<SearchFixture, Box<dyn std::error::Error>> {
    let plan = plan()?;
    let candidate = candidate()?;
    let mut trace = SearchTrace::from_plan(
        &plan,
        vec!["fixture-retriever".to_string()],
        std::slice::from_ref(&candidate),
        vec![
            SearchTraceFilter::Acl,
            SearchTraceFilter::Quarantine,
            SearchTraceFilter::PromptInjection,
            SearchTraceFilter::Freshness,
            SearchTraceFilter::Trust,
            SearchTraceFilter::Sensitivity,
        ],
        Some("rrf-fixed-k60".to_string()),
        Vec::new(),
        SearchStopReason::EvidenceComplete,
    );
    trace = trace.with_policy_fingerprint(
        "trust=Some(Untrusted);sensitivity=Some(Internal);read_allowed=true;scope=None;unscoped=true"
            .to_string(),
    );
    let outcome = SearchOutcome {
        trace: trace.deterministic_id(),
        trace_data: Some(Box::new(trace)),
        fingerprint: plan.fingerprint.clone(),
        index_generation: plan.index_generation,
        status: SearchStatus::Answerable,
        evidence: vec![candidate],
        coverage: EvidenceCoverage {
            percent_covered: 100,
            gaps_identified: Vec::new(),
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            distinct_sources: 0,
            distinct_documents: 0,
            distinct_sections: 0,
            candidate_coverage_keys: Vec::new(),
        },
        conflicts: Vec::new(),
    };
    Ok(SearchFixture {
        plan,
        outcome,
        evidences: BTreeMap::from([(EvidenceId::new(10), evidence())]),
        artifacts: BTreeMap::from([(
            ArtifactId::new(12),
            Artifact {
                id: ArtifactId::new(12),
                title: "fixture".to_string(),
                chunk_ids: BTreeSet::new(),
                card_ids: BTreeSet::new(),
                claim_ids: BTreeSet::new(),
                evidence_ids: BTreeSet::from([EvidenceId::new(10)]),
                index_status: IndexStatus::Indexed,
                content_hash: None,
                parse_status: None,
                security: SecurityMetadata::default(),
            },
        )]),
        claims: BTreeMap::new(),
        memory_candidates: BTreeMap::new(),
    })
}

pub fn claim(id: u64, evidence_ids: impl IntoIterator<Item = EvidenceId>) -> Claim {
    Claim {
        id: ClaimId::new(id),
        artifact_id: ArtifactId::new(id),
        text: format!("claim {id}"),
        status: ClaimStatus::Proposed,
        evidence_ids: evidence_ids.into_iter().collect(),
        security: SecurityMetadata::default(),
    }
}
