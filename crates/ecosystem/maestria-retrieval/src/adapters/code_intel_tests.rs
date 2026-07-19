use super::*;
use maestria_code_intel::RepositoryIdentitySnapshot;
use maestria_domain::{
    CorpusScope, EvidenceRequirements, FreshnessRequirement, IndexGenerationId, Modality,
    ModalitySet, QueryId, RetrievalModelFingerprint, SearchBudget, SearchCompatibilityError,
    SearchIntent, SearchPlan, SearchStage, SourceLocation, StopConditions,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::SearchQuery;

fn archive() -> maestria_code_intel::RepositoryCodeIndex {
    maestria_code_intel::RepositoryCodeIndex {
        summary: maestria_code_intel::CodeIndexSummary {
            repository_root: "/root/repo".to_string(),
            commit_sha: "abc123".to_string(),
            worktree_identity: "wt-1".to_string(),
            parser_generation: "cargo-rust-code-v1".to_string(),
            package_count: 1,
            target_count: 1,
            symbol_count: 1,
            file_count: 1,
            packages: vec!["pkg".to_string()],
            excluded_patterns: Vec::new(),
            relation_summary: maestria_code_intel::CodeRelationSummary::default(),
        },
        packages: Vec::new(),
        symbols: vec![symbol("rec-1")],
        relations: Vec::new(),
    }
}

fn symbol(record_id: &str) -> SymbolRecord {
    SymbolRecord {
        record_id: record_id.to_string(),
        package: "pkg".to_string(),
        target: "main".to_string(),
        kind: maestria_code_intel::SymbolKind::Function,
        name: "compute".to_string(),
        qualified_name: "crate::compute".to_string(),
        visibility: maestria_code_intel::Visibility::Public,
        is_public_api: true,
        is_async: false,
        is_unsafe: false,
        is_test: false,
        is_bench: false,
        signature: None,
        imports: Vec::new(),
        markers: maestria_code_intel::SymbolMarkers::default(),
        provenance: maestria_code_intel::RecordProvenance {
            repository_root: "/root/repo".to_string(),
            commit_sha: "abc123".to_string(),
            worktree_identity: "wt-1".to_string(),
            file_path: "src/lib.rs".to_string(),
            source_range: maestria_code_intel::SourceRange {
                start_line: 10,
                end_line: 15,
            },
            parser_generation: "cargo-rust-code-v1".to_string(),
        },
    }
}

fn plan() -> Result<SearchPlan, SearchCompatibilityError> {
    let mut plan = SearchPlan {
        query_id: QueryId::new(1),
        original_query: "compute".to_string(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: maestria_domain::CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Code]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(100, 300)?,
        stop_conditions: StopConditions {
            max_results: 10,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        },
        fingerprint: RetrievalModelFingerprint::new("maestria:test".into())?,
    };
    plan.budgets = SearchBudget::with_limits(100, 300, 10, 1, 0)?;
    Ok(plan)
}

fn candidate_request(
    expected_generation: IndexGenerationId,
    query: &str,
    limit: usize,
) -> Result<CandidateRequest, Box<dyn std::error::Error>> {
    Ok(CandidateRequest {
        plan: plan()?,
        query: SearchQuery {
            q: query.to_string(),
            limit,
            offset: 0,
        },
        expected_generation,
    })
}

fn retriever(generation: IndexGenerationId) -> CodeIntelRetriever {
    CodeIntelRetriever::new(
        CodeIntelRetrieverParts {
            index: Arc::new(archive()),
        },
        RetrievalSecurityPolicy::default(),
        generation,
    )
}

#[tokio::test]
async fn rejects_generation_mismatch() -> Result<(), Box<dyn std::error::Error>> {
    let retriever = retriever(IndexGenerationId::new(2));
    let request = candidate_request(IndexGenerationId::new(1), "compute", 5)?;
    assert!(matches!(
        retriever.retrieve(request).await,
        Err(RetrievalError::Internal(_))
    ));
    Ok(())
}

#[test]
fn maps_current_and_stale_freshness_honestly() {
    let current = freshness_status_to_domain(RepositoryFreshness::Current {
        indexed: RepositoryIdentitySnapshot {
            commit_sha: "a".to_string(),
            worktree_identity: "w".to_string(),
        },
        current: RepositoryIdentitySnapshot {
            commit_sha: "a".to_string(),
            worktree_identity: "w".to_string(),
        },
    });
    let stale = freshness_status_to_domain(RepositoryFreshness::Stale {
        indexed: RepositoryIdentitySnapshot {
            commit_sha: "a".to_string(),
            worktree_identity: "w".to_string(),
        },
        current: RepositoryIdentitySnapshot {
            commit_sha: "b".to_string(),
            worktree_identity: "x".to_string(),
        },
    });
    assert_eq!(current, FreshnessStatus::UpToDate);
    assert_eq!(stale, FreshnessStatus::Stale);
}

#[test]
fn candidate_ids_are_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let retriever = retriever(IndexGenerationId::new(1));
    let symbol = symbol("rec-1");
    let candidate_a = retriever.candidate_from_symbol(&symbol, FreshnessStatus::UpToDate, 0)?;
    let candidate_b = retriever.candidate_from_symbol(&symbol, FreshnessStatus::UpToDate, 0)?;
    assert_eq!(candidate_a.evidence_id, candidate_b.evidence_id);
    assert_eq!(candidate_a.artifact_version, candidate_b.artifact_version);
    Ok(())
}

#[test]
fn candidate_includes_expected_code_source_provenance() -> Result<(), Box<dyn std::error::Error>> {
    let retriever = retriever(IndexGenerationId::new(1));
    let symbol = symbol("rec-1");
    let candidate = retriever.candidate_from_symbol(&symbol, FreshnessStatus::UpToDate, 3)?;
    assert_eq!(
        candidate.source_span.location(),
        &SourceLocation::File {
            path: "src/lib.rs".to_string(),
            start_line: 10,
            end_line: 15
        }
    );
    assert_eq!(candidate.source_span.range().start, 10);
    assert_eq!(candidate.source_span.range().end, 15);
    assert_eq!(candidate.freshness, FreshnessStatus::UpToDate);
    assert_eq!(
        candidate.coverage_keys,
        vec!["symbol:rec-1", "file:src/lib.rs"]
    );
    Ok(())
}
