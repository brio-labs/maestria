use std::collections::{BTreeMap, BTreeSet};

use maestria_domain::{
    Artifact, ArtifactId, ArtifactVersionId, BlobId, Claim, ClaimId, ClaimStatus, ContentRange,
    CorpusScope, Evidence, EvidenceCandidate, EvidenceCoverage, EvidenceId, EvidenceKind,
    EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus, IndexGenerationId,
    IndexStatus, LogicalTick, Modality, ModalitySet, QueryId, RetrievalModelFingerprint,
    RetrievalReason, RetrievalScoreSet, SearchBudget, SearchIntent, SearchOutcome, SearchPlan,
    SearchStage, SearchStatus, SearchStopReason, SearchTrace, SearchTraceFilter, SecurityMetadata,
    SourceLocation, StopConditions, TrustLabel, TrustZone, ValidationReportId,
};

use super::{
    CandidateProvenanceValidator, CitationAlignmentValidator, ConflictValidator, CoverageValidator,
    FreshnessValidator, RetrievalSecurityValidator, SearchPlanValidator, SearchRegressionValidator,
    SearchValidationContext, ValidationContext, ValidationRunner, Validator,
};

struct SearchFixture {
    plan: SearchPlan,
    outcome: SearchOutcome,
    evidences: BTreeMap<EvidenceId, Evidence>,
    artifacts: BTreeMap<ArtifactId, Artifact>,
    claims: BTreeMap<maestria_domain::ClaimId, maestria_domain::Claim>,
    memory_candidates:
        BTreeMap<maestria_domain::MemoryCandidateId, maestria_domain::MemoryCandidate>,
}

impl SearchFixture {
    fn context(&self) -> ValidationContext<'_> {
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

fn plan() -> Result<SearchPlan, Box<dyn std::error::Error>> {
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

fn candidate() -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
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
        scores: RetrievalScoreSet {
            bm25: 900,
            semantic_similarity: 800,
        },
        trust: TrustLabel::Unverified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: None,
        reasons: vec![RetrievalReason::ExactMatch],
        coverage_keys: Vec::new(),
    })
}

fn evidence() -> Evidence {
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
fn fixture() -> Result<SearchFixture, Box<dyn std::error::Error>> {
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
fn claim(id: u64, evidence_ids: impl IntoIterator<Item = EvidenceId>) -> Claim {
    Claim {
        id: ClaimId::new(id),
        artifact_id: ArtifactId::new(id),
        text: format!("claim {id}"),
        status: ClaimStatus::Proposed,
        evidence_ids: evidence_ids.into_iter().collect(),
        security: SecurityMetadata::default(),
    }
}

#[test]
fn all_search_validators_execute_and_pass_for_a_reproducible_outcome()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let validators: Vec<Box<dyn Validator>> = vec![
        Box::new(SearchPlanValidator),
        Box::new(CandidateProvenanceValidator),
        Box::new(CoverageValidator),
        Box::new(ConflictValidator),
        Box::new(FreshnessValidator),
        Box::new(CitationAlignmentValidator),
        Box::new(RetrievalSecurityValidator),
        Box::new(SearchRegressionValidator),
    ];
    let report = ValidationRunner::with_validators(validators).run(
        ValidationReportId::new(1),
        None,
        &fixture.context(),
    );

    assert!(report.passed, "{:?}", report.checks);
    assert_eq!(report.checks.len(), 8);
    assert!(report.checks.iter().all(|check| check.passed));
    assert!(
        report
            .checks
            .iter()
            .all(|check| matches!(check.severity, crate::Severity::Error))
    );
    Ok(())
}

#[test]
fn search_plan_validator_fails_with_invalid_plan_schema() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = fixture()?;
    fixture.plan.evidence_requirements.minimum_corroboration = 0;
    let check = SearchPlanValidator.validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("minimum corroboration"));
    Ok(())
}

#[test]
fn candidate_provenance_validator_fails_for_missing_evidence_record()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture()?;
    fixture.evidences.clear();
    let check = CandidateProvenanceValidator.validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("invalid evidence record"));
    Ok(())
}

#[test]
fn coverage_validator_fails_when_answerable_coverage_is_incomplete()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture()?;
    fixture.outcome.coverage.percent_covered = 50;
    let check = CoverageValidator.validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("Answerable"));
    Ok(())
}

#[test]
fn conflict_validator_fails_when_status_and_members_mismatch()
-> Result<(), Box<dyn std::error::Error>> {
    let mut conflict_fixture = fixture()?;
    conflict_fixture.outcome.status = SearchStatus::SourcesConflict;
    let check = ConflictValidator.validate(&conflict_fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("disagree"));

    let mut unknown_fixture = fixture()?;
    let unknown_conflict = maestria_domain::ConflictSet {
        id: maestria_domain::ConflictSetId::new(10),
        candidates: vec![conflict_fixture.outcome.evidence[0].clone()],
    };
    unknown_fixture.outcome.status = SearchStatus::Answerable;
    unknown_fixture.outcome.conflicts.push(unknown_conflict);
    let check = ConflictValidator.validate(&unknown_fixture.context());
    assert!(!check.passed);
    Ok(())
}

#[test]
fn freshness_validator_fails_for_stale_high_rank_evidence() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = fixture()?;
    if let Some(candidate) = fixture.outcome.evidence.first_mut() {
        candidate.freshness = FreshnessStatus::Stale;
    }
    let check = FreshnessValidator.validate(&fixture.context());
    assert!(!check.passed);
    Ok(())
}

#[test]
fn citation_alignment_validator_fails_for_unbound_claims() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = fixture()?;
    fixture
        .claims
        .insert(ClaimId::new(1), claim(1, [EvidenceId::new(99)]));
    let check = CitationAlignmentValidator.validate(&fixture.context());
    assert!(!check.passed);
    Ok(())
}

#[test]
fn retrieval_security_validator_requires_required_filters() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = fixture()?;
    if let Some(trace) = fixture.outcome.trace_data.as_mut() {
        trace.filters = vec![SearchTraceFilter::Acl, SearchTraceFilter::Quarantine];
    }
    let check = RetrievalSecurityValidator.validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("required filter"));
    Ok(())
}

type SecurityMutation = fn(&mut Evidence);
#[test]
fn security_validator_blocks_poisoning_prompt_injection_secret_acl_and_quarantine()
-> Result<(), Box<dyn std::error::Error>> {
    fn poison(evidence: &mut Evidence) {
        evidence
            .security
            .poisoning_flags
            .push("graph_poisoning".to_string());
    }
    fn prompt_injection(evidence: &mut Evidence) {
        evidence.security.prompt_injection_risk = true;
    }
    fn secret(evidence: &mut Evidence) {
        evidence
            .security
            .poisoning_flags
            .push("secret_signal".to_string());
    }
    fn acl(evidence: &mut Evidence) {
        evidence.security.read_allowed = false;
    }
    fn quarantine(evidence: &mut Evidence) {
        evidence.security.quarantined = true;
        evidence.security.trust_zone = TrustZone::Quarantined;
    }

    let cases: [(&str, SecurityMutation); 5] = [
        ("poisoning", poison),
        ("prompt injection", prompt_injection),
        ("secret", secret),
        ("acl", acl),
        ("quarantine", quarantine),
    ];
    for (label, mutate) in cases {
        let mut fixture = fixture()?;
        let Some(evidence) = fixture.evidences.get_mut(&EvidenceId::new(10)) else {
            return Err(format!("fixture lost evidence for {label}").into());
        };
        mutate(evidence);
        let check = RetrievalSecurityValidator.validate(&fixture.context());
        assert!(!check.passed, "security case should fail: {label}");
        assert!(check.message.contains("1 denied candidate(s)"));
    }
    Ok(())
}

#[test]
fn security_validator_enforces_typed_policy_values() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture()?;
    if let Some(trace) = fixture.outcome.trace_data.as_mut() {
        trace.policy_fingerprint = Some(
            "trust=Some(Verified);sensitivity=Some(Public);read_allowed=true;scope=Some(ScopeId(999));unscoped=false"
                .to_string(),
        );
        trace.filters = vec![
            SearchTraceFilter::Acl,
            SearchTraceFilter::Trust,
            SearchTraceFilter::Sensitivity,
            SearchTraceFilter::Scope,
            SearchTraceFilter::Quarantine,
            SearchTraceFilter::PromptInjection,
            SearchTraceFilter::Freshness,
        ];
    }
    let check = RetrievalSecurityValidator.validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("denied candidate"));
    assert!(check.message.contains("1 denied candidate(s)"));
    Ok(())
}

#[test]
fn search_regression_validator_fails_for_identity_and_duplicate_candidates()
-> Result<(), Box<dyn std::error::Error>> {
    let mut identity_fixture = fixture()?;
    identity_fixture.outcome.trace = maestria_domain::SearchTraceId::new(404);
    let check = SearchRegressionValidator.validate(&identity_fixture.context());
    assert!(!check.passed);
    let mut duplicate_fixture = fixture()?;
    if let Some(first) = duplicate_fixture.outcome.evidence.first().cloned() {
        duplicate_fixture.outcome.evidence.push(first);
        let duplicate_check = SearchRegressionValidator.validate(&duplicate_fixture.context());
        assert!(!duplicate_check.passed);
    }
    Ok(())
}
