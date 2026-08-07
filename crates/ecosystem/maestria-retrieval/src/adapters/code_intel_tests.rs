use super::*;
use crate::adapters::CodeIntelSecurityResolverParts;
use crate::types::CandidateSourceFilter;
use maestria_code_intel::SymbolRecord;
use maestria_domain::{
    ArtifactId, ArtifactVersionId, BlobId, ContentHash, CorpusScope, Evidence, EvidenceId,
    EvidenceKind, EvidenceRequirements, FreshnessRequirement, IndexGenerationId, LineRange,
    LogicalTick, Modality, ModalitySet, QueryId, RetrievalModelFingerprint, SearchBudget,
    SearchCompatibilityError, SearchIntent, SearchPlan, SearchStage, SecurityMetadata, SnapshotRef,
    SourceLocation, StopConditions,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryEvidenceRepository, SearchQuery,
};
use std::collections::BTreeSet;

fn archive() -> Result<maestria_code_intel::RepositoryCodeIndex, Box<dyn std::error::Error>> {
    Ok(maestria_code_intel::RepositoryCodeIndex {
        summary: maestria_code_intel::CodeIndexSummary {
            repository_root: "/root/repo".to_string(),
            commit_sha: maestria_code_intel::CommitSha::new("abc123"),
            worktree_identity: maestria_code_intel::WorktreeIdentity::new("wt-1"),
            parser_generation: maestria_code_intel::ParserGeneration::new("cargo-rust-code-v3"),
            package_count: 1,
            target_count: 1,
            symbol_count: 1,
            file_count: 1,
            packages: vec!["pkg".to_string()],
            excluded_patterns: Vec::new(),
            workspace_warnings: Vec::new(),
            relation_summary: maestria_code_intel::CodeRelationSummary::default(),
            changed: maestria_code_intel::RepositoryChangeDelta {
                files: Vec::new(),
                symbols: Vec::new(),
            },
        },
        packages: Vec::new(),
        symbols: vec![symbol("rec-1")?],
        relations: Vec::new(),
        file_contexts: std::collections::BTreeMap::new(),
    })
}

fn symbol(record_id: &str) -> Result<SymbolRecord, Box<dyn std::error::Error>> {
    Ok(SymbolRecord {
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
        doc_comment: None,
        markers: maestria_code_intel::SymbolMarkers::default(),
        provenance: maestria_code_intel::RecordProvenance {
            repository_root: "/root/repo".to_string(),
            commit_sha: maestria_code_intel::CommitSha::new("abc123"),
            worktree_identity: maestria_code_intel::WorktreeIdentity::new("wt-1"),
            content_hash: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            file_path: "src/lib.rs".to_string(),
            source_range: maestria_code_intel::SourceRange::new(10, 15)?,
            parser_generation: maestria_code_intel::ParserGeneration::new("cargo-rust-code-v3"),
        },
    })
}

fn plan() -> Result<SearchPlan, SearchCompatibilityError> {
    SearchPlan::builder()
        .query_id(QueryId::new(1))
        .original_query("compute".to_string())
        .intent(SearchIntent::FactualLocal)
        .scope(CorpusScope::Global)
        .corpus_snapshot(maestria_domain::CorpusSnapshotId::new(1))
        .index_generation(IndexGenerationId::new(1))
        .freshness(FreshnessRequirement::Any)
        .modalities(ModalitySet::new(vec![Modality::Code]))
        .stages(vec![SearchStage::InitialRetrieval])
        .budgets(SearchBudget::with_limits(100, 300, 10, 1, 0)?)
        .stop_conditions(StopConditions {
            max_results: 10,
            min_score_threshold: 0,
        })
        .evidence_requirements(EvidenceRequirements {
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        })
        .fingerprint(RetrievalModelFingerprint::new("maestria:test".into())?)
        .authorization(maestria_domain::RetrievalPolicySnapshot::global_default())
        .build()
}

fn candidate_request(
    expected_generation: IndexGenerationId,
    query: &str,
    limit: usize,
) -> Result<CandidateRequest, Box<dyn std::error::Error>> {
    let plan = plan()?;
    let authorization = RetrievalSecurityPolicy::default().authorization_context(plan.scope())?;
    let execution_budget = plan.execution_budget()?;
    Ok(CandidateRequest {
        plan,
        query: SearchQuery {
            q: query.to_string(),
            limit,
            offset: 0,
            execution_budget,
        },
        execution_budget,
        expected_generation,
        authorization,
        source_filter: None,
    })
}

fn retriever(
    generation: IndexGenerationId,
) -> Result<CodeIntelRetriever, Box<dyn std::error::Error>> {
    let security = CodeIntelSecurityResolver::from_events(
        CodeIntelSecurityResolverParts {
            artifacts: Arc::new(InMemoryArtifactRepository::new()),
            evidence: Arc::new(InMemoryEvidenceRepository::new()),
            blobs: Arc::new(InMemoryBlobStore::new()),
        },
        &[],
    )?;
    Ok(CodeIntelRetriever::new(
        CodeIntelRetrieverParts {
            index: Arc::new(archive()?),
            security,
        },
        generation,
    ))
}

fn authorized_binding() -> Result<AuthorizedCodeBinding, Box<dyn std::error::Error>> {
    let content_hash = ContentHash::new(
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
    )?;
    Ok(AuthorizedCodeBinding {
        symbol: symbol("rec-1")?,
        artifact_version: ArtifactVersionId::new(73),
        evidence: Evidence {
            id: EvidenceId::new(79),
            artifact_id: ArtifactId::new(71),
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/root/repo/src/lib.rs".to_string(),
                range: LineRange::new(10, 15)?,
                snapshot: SnapshotRef::new(BlobId::new(77), content_hash),
            },
            excerpt: "fn compute() {}".to_string(),
            observed_at: LogicalTick::new(1),
            security: SecurityMetadata::default(),
        },
        security: SecurityMetadata::default(),
    })
}

#[tokio::test]
async fn rejects_generation_mismatch() -> Result<(), Box<dyn std::error::Error>> {
    let retriever = retriever(IndexGenerationId::new(2))?;
    let request = candidate_request(IndexGenerationId::new(1), "compute", 5)?;
    assert!(matches!(
        retriever.retrieve(request).await,
        Err(RetrievalError::Internal(_))
    ));
    Ok(())
}

#[test]
fn candidate_ids_are_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let retriever = retriever(IndexGenerationId::new(1))?;
    let candidate_a = retriever.candidate_from_binding(
        authorized_binding()?,
        FreshnessStatus::UpToDate,
        None,
        0,
    )?;
    let candidate_b = retriever.candidate_from_binding(
        authorized_binding()?,
        FreshnessStatus::UpToDate,
        None,
        0,
    )?;
    assert_eq!(candidate_a.evidence_id(), candidate_b.evidence_id());
    assert_eq!(
        candidate_a.artifact_version(),
        candidate_b.artifact_version()
    );
    assert_eq!(candidate_a.evidence_id(), EvidenceId::new(79));
    assert_eq!(candidate_a.artifact_version(), ArtifactVersionId::new(73));
    Ok(())
}

#[test]
fn candidate_includes_expected_code_source_provenance() -> Result<(), Box<dyn std::error::Error>> {
    let retriever = retriever(IndexGenerationId::new(1))?;
    let candidate = retriever.candidate_from_binding(
        authorized_binding()?,
        FreshnessStatus::UpToDate,
        Some(&RepositoryIdentitySnapshot {
            commit_sha: maestria_code_intel::CommitSha::new("live-commit"),
            worktree_identity: maestria_code_intel::WorktreeIdentity::new("live-worktree"),
        }),
        3,
    )?;
    assert_eq!(
        candidate.source_span().location(),
        &SourceLocation::File {
            path: "/root/repo/src/lib.rs".to_string(),
            start_line: 10,
            end_line: 15
        }
    );
    assert_eq!(candidate.source_span().range().start(), 10);
    assert_eq!(candidate.source_span().range().end(), 15);
    assert_eq!(candidate.freshness(), FreshnessStatus::UpToDate);
    assert_eq!(
        candidate.coverage_keys(),
        vec!["symbol:rec-1", "file:src/lib.rs"]
    );
    // The retrieval-time freshness read must be preserved as evidence in the
    // candidate provenance (R51), distinct from the indexed identity.
    let components = &candidate.scores().lanes()[0].fingerprint.components;
    assert_eq!(
        components.get("observed_commit_sha"),
        Some(&"live-commit".to_string())
    );
    assert_eq!(
        components.get("observed_worktree_identity"),
        Some(&"live-worktree".to_string())
    );
    Ok(())
}

#[test]
fn source_filter_rejects_disallowed_code_binding() -> Result<(), Box<dyn std::error::Error>> {
    let retriever = retriever(IndexGenerationId::new(1))?;
    let mut request = candidate_request(IndexGenerationId::new(1), "compute", 5)?;
    request.source_filter = Some(CandidateSourceFilter::try_new(BTreeSet::from([
        ArtifactId::new(72),
    ]))?);
    let query_result = QueryResult {
        summary: maestria_code_intel::QuerySummary {
            query: maestria_code_intel::CodeQuery::Symbol {
                pattern: "compute".to_string(),
            },
            matched: 1,
            returned: 1,
            truncated: false,
            scanned: 1,
            limit: 5,
            regex_error: None,
        },
        records: vec![symbol("rec-1")?],
    };
    let (candidates, _, _) = retriever.materialize_candidates(
        &request,
        query_result,
        vec![authorized_binding()?],
        FreshnessStatus::UpToDate,
        None,
    )?;
    assert!(candidates.is_empty());
    Ok(())
}
