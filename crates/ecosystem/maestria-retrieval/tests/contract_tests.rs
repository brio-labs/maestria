use maestria_domain::{
    ArtifactVersionId, ContentRange, CorpusScope, CorpusSnapshotId, EvidenceCandidate,
    EvidenceCoverage, EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus,
    IndexGenerationId, ModalitySet, QueryId, RetrievalModelFingerprint, RetrievalReason,
    RetrievalScoreSet, SearchBudget, SearchIntent, SearchOutcome, SearchPlan, SearchStage,
    SearchStatus, SearchTraceId, SourceLocation, StopConditions, StructureNodeId, TrustLabel,
};
use maestria_retrieval::{RetrievalError, RetrievalResult, engine::SyncRetrievalEngine};

fn dummy_plan() -> RetrievalResult<SearchPlan> {
    Ok(SearchPlan {
        query_id: QueryId::new(1),
        original_query: "test query".to_string(),
        intent: SearchIntent::ExactLookup,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(1000, 100)?,
        stop_conditions: StopConditions {
            max_results: 10,
            min_score_threshold: 50,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
        },
        fingerprint: RetrievalModelFingerprint::new("dummy-model".into())?,
    })
}

fn dummy_outcome() -> RetrievalResult<SearchOutcome> {
    Ok(SearchOutcome {
        trace: SearchTraceId::new(1),
        fingerprint: RetrievalModelFingerprint::new("dummy-model".into())?,
        index_generation: IndexGenerationId::new(1),
        status: SearchStatus::Answerable,
        evidence: vec![],
        coverage: EvidenceCoverage {
            percent_covered: 100,
            gaps_identified: vec![],
        },
        conflicts: vec![],
    })
}

fn candidate_fixture() -> RetrievalResult<EvidenceCandidate> {
    Ok(EvidenceCandidate {
        evidence_id: maestria_domain::EvidenceId::new(23),
        artifact_version: ArtifactVersionId::new(19),
        source_span: EvidenceSpan::new(
            Some(StructureNodeId::new(29)),
            SourceLocation::File {
                path: "notes/research.md".to_string(),
                start_line: 4,
                end_line: 8,
            },
            ContentRange { start: 32, end: 96 },
        )?,
        scores: RetrievalScoreSet {
            bm25: 91,
            semantic_similarity: 88,
        },
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(maestria_domain::DuplicateClusterId::new(31)),
        reasons: vec![RetrievalReason::ExactMatch, RetrievalReason::CitationLink],
    })
}

fn one_candidate(_: &SearchPlan) -> RetrievalResult<Vec<EvidenceCandidate>> {
    Ok(vec![candidate_fixture()?])
}
#[test]
fn test_sync_engine_orchestration() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let outcome = dummy_outcome()?;
    let retriever = |_: &SearchPlan| -> RetrievalResult<Vec<EvidenceCandidate>> { Ok(vec![]) };
    let evaluator = move |_: Vec<EvidenceCandidate>, _: &SearchPlan| Ok(outcome.clone());
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    let result = engine.search_sync(&plan)?;
    assert_eq!(result.status, SearchStatus::Answerable);
    Ok(())
}

#[test]
fn test_timeout_cancellation() -> RetrievalResult<()> {
    let mut plan = dummy_plan()?;
    plan.budgets = SearchBudget::new(1000, 1)?;
    let retriever = |_: &SearchPlan| -> RetrievalResult<Vec<EvidenceCandidate>> {
        std::thread::sleep(std::time::Duration::from_millis(10));
        Ok(vec![])
    };
    let evaluator = move |_: Vec<EvidenceCandidate>, _: &SearchPlan| dummy_outcome();
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    let result = engine.search_sync(&plan);
    assert!(matches!(result, Err(RetrievalError::Timeout)));
    Ok(())
}

#[test]
fn test_fingerprint_compatibility_pass() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let retriever = |p: &SearchPlan| -> RetrievalResult<Vec<EvidenceCandidate>> {
        if p.fingerprint.as_str() != "dummy-model" {
            return Err(RetrievalError::Internal("fingerprint mismatch".into()));
        }
        Ok(vec![])
    };
    let evaluator = move |_: Vec<EvidenceCandidate>, p: &SearchPlan| {
        let mut out = dummy_outcome()?;
        out.fingerprint = p.fingerprint.clone();
        Ok(out)
    };
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    assert!(engine.search_sync(&plan).is_ok());
    Ok(())
}

#[test]
fn test_scope_acl_filtering() -> RetrievalResult<()> {
    let mut plan = dummy_plan()?;
    plan.scope = CorpusScope::Restricted(vec![]);
    let retriever = |p: &SearchPlan| -> RetrievalResult<Vec<EvidenceCandidate>> {
        match p.scope {
            CorpusScope::Restricted(_) => Ok(vec![]),
            _ => Err(RetrievalError::Internal("unexpected scope".into())),
        }
    };
    let evaluator = move |_: Vec<EvidenceCandidate>, _: &SearchPlan| dummy_outcome();
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    assert!(engine.search_sync(&plan).is_ok());
    Ok(())
}

#[test]
fn test_provenance_scores_reasons_and_determinism() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let candidate = candidate_fixture()?;
    let retriever = move |_: &SearchPlan| Ok(vec![candidate.clone()]);
    let evaluator = move |candidates: Vec<EvidenceCandidate>, plan: &SearchPlan| {
        let mut outcome = dummy_outcome()?;
        outcome.evidence = candidates;
        outcome.fingerprint = plan.fingerprint.clone();
        outcome.status = SearchStatus::Answerable;
        Ok(outcome)
    };
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    let first = engine.search_sync(&plan)?;
    let second = engine.search_sync(&plan)?;
    assert_eq!(first, second);
    let evidence = first.evidence.first().ok_or(RetrievalError::Internal(
        "candidate was not preserved".into(),
    ))?;
    assert_eq!(evidence.scores.bm25, 91);
    assert_eq!(
        evidence.reasons,
        vec![RetrievalReason::ExactMatch, RetrievalReason::CitationLink]
    );
    assert_eq!(
        evidence.source_span.range(),
        ContentRange { start: 32, end: 96 }
    );
    Ok(())
}

#[test]
fn test_missing_artifacts_return_no_evidence() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let retriever = |_: &SearchPlan| Ok(Vec::<EvidenceCandidate>::new());
    let evaluator = move |candidates: Vec<EvidenceCandidate>, plan: &SearchPlan| {
        let mut outcome = dummy_outcome()?;
        outcome.evidence = candidates;
        outcome.status = SearchStatus::NoEvidenceFound;
        outcome.fingerprint = plan.fingerprint.clone();
        outcome.coverage.percent_covered = 0;
        Ok(outcome)
    };
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    let outcome = engine.search_sync(&plan)?;
    assert_eq!(outcome.status, SearchStatus::NoEvidenceFound);
    assert!(outcome.evidence.is_empty());
    Ok(())
}

#[test]
fn test_sync_pipeline_bounds_multiple_retrievers() -> RetrievalResult<()> {
    let mut plan = dummy_plan()?;
    plan.stop_conditions.max_results = 1;
    let evaluator = |candidates: Vec<EvidenceCandidate>, plan: &SearchPlan| {
        assert!(candidates.len() <= plan.stop_conditions.max_results as usize);
        let mut outcome = dummy_outcome()?;
        outcome.evidence = candidates;
        outcome.fingerprint = plan.fingerprint.clone();
        Ok(outcome)
    };
    let engine = SyncRetrievalEngine::new(vec![one_candidate, one_candidate], evaluator);
    let outcome = engine.search_sync(&plan)?;
    assert_eq!(outcome.evidence.len(), 1);
    Ok(())
}
