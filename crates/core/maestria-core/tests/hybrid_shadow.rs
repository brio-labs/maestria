use maestria_core::{CorePorts, CoreServices, SearchInput};
use maestria_domain::{
    Artifact, ArtifactId, Chunk, Evidence, EvidenceKind, IndexStatus, SourceSpan, StructureNodeId,
};
use maestria_domain::{
    CorpusScope, CorpusSnapshotId, EvidenceRequirements, FreshnessRequirement, IndexGenerationId,
    Modality, ModalitySet, QueryId, RetrievalModelFingerprint, SearchBudget, SearchIntent,
    SearchPlan, SearchStage, StopConditions,
};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, InMemoryArtifactRepository,
    InMemoryBlobStore, InMemoryChunkRepository, InMemoryEventLog, InMemoryEvidenceRepository,
    InMemoryFullTextIndex, InMemoryParser, InMemoryVectorIndex, VectorIndex, VectorSearchQuery,
};

type VectorFixture = (
    InMemoryArtifactRepository,
    InMemoryChunkRepository,
    InMemoryEvidenceRepository,
    InMemoryBlobStore,
    InMemoryEventLog,
    InMemoryParser,
    InMemoryFullTextIndex,
    InMemoryVectorIndex,
    maestria_domain::ArtifactId,
    maestria_domain::ChunkId,
    maestria_domain::EvidenceId,
);

fn seed_vector_fixture() -> Result<VectorFixture, Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(800);
    let chunk_id = maestria_domain::ChunkId::new(801);
    let evidence_id = maestria_domain::evidence_id_for(artifact_id, 0);
    let source = "literal source text\n";
    let artifact_repo = InMemoryArtifactRepository::new();
    let chunk_repo = InMemoryChunkRepository::new();
    let evidence_repo = InMemoryEvidenceRepository::new();
    let blob_store = InMemoryBlobStore::new();
    let events = InMemoryEventLog::new();
    let parser = InMemoryParser::new();
    let search_index = InMemoryFullTextIndex::new();
    let vector_index = InMemoryVectorIndex::new();

    let blob_id = blob_store.put(source.as_bytes().to_vec())?;
    artifact_repo.put(Artifact {
        id: artifact_id,
        title: "semantic.md".to_string(),
        chunk_ids: [chunk_id].into(),
        security: maestria_domain::SecurityMetadata::default(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: [evidence_id].into(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(maestria_core::content_hash(source.as_bytes())),
        parse_status: None,
    })?;
    chunk_repo.put(Chunk {
        id: chunk_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        order: 0,
        text: "semantic token".to_string(),
    })?;
    evidence_repo.put(Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "semantic.md".to_string(),
            range: maestria_domain::ContentRange { start: 1, end: 1 },
            content_hash: maestria_core::content_hash(source.as_bytes()),
            snapshot: Some(blob_id),
        },
        excerpt: "literal source text".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    })?;
    vector_index.index_embeddings(vec![maestria_ports::VectorEmbedding {
        chunk_id,
        vector: vec![0.0, 1.0],
        provenance: maestria_ports::EmbeddingProvenance {
            content_hash: "hash".to_string(),
            identity: maestria_ports::EmbeddingIdentity::legacy("test-model", 2)?,
            provider_id: "test-provider".to_string(),
            model: "test-model".to_string(),
            model_version: "test-v1".to_string(),
            disclosure: maestria_ports::ProviderDisclosure {
                remote: false,
                retention: maestria_ports::RetentionPolicy::NoRetention,
            },
        },
    }])?;
    Ok((
        artifact_repo,
        chunk_repo,
        evidence_repo,
        blob_store,
        events,
        parser,
        search_index,
        vector_index,
        artifact_id,
        chunk_id,
        evidence_id,
    ))
}

fn search_with_policy(
    policy: maestria_core::HybridExecutionPolicy,
) -> Result<maestria_core::SearchOutput, Box<dyn std::error::Error>> {
    let (
        artifact_repo,
        chunk_repo,
        evidence_repo,
        blob_store,
        events,
        parser,
        search_index,
        vector_index,
        _artifact_id,
        _chunk_id,
        _evidence_id,
    ) = seed_vector_fixture()?;
    let card_repo = maestria_ports::InMemoryCardRepository::new();
    let core = CoreServices::new(CorePorts {
        artifacts: &artifact_repo,
        chunks: &chunk_repo,
        cards: &card_repo,
        evidence: &evidence_repo,
        events: &events,
        parser: &parser,
        search_index: &search_index,
        blobs: &blob_store,
        vector_index: Some(&vector_index),
        graph_index: None,
    })
    .with_hybrid_policy(policy);
    let output = core.search_with_vector(
        SearchInput {
            query: "unrelated query".to_string(),
            limit: 5,
        },
        VectorSearchQuery {
            vector: vec![0.0, 1.0],
            limit: 5,
            provider_id: Some("test-provider".to_string()),
            model: Some("test-model".to_string()),
            model_version: Some("test-v1".to_string()),
            identity: None,
        },
    )?;
    Ok(output)
}

#[test]
fn shadow_executes_dense_lane_but_suppresses_fusion() -> Result<(), Box<dyn std::error::Error>> {
    let output = search_with_policy(maestria_core::HybridExecutionPolicy::Shadow)?;
    assert_eq!(output.mode, maestria_core::RetrievalMode::HybridShadow);
    let dense_report = output
        .lane_reports
        .iter()
        .find(|report| report.retriever_id == "dense_chunks")
        .ok_or("dense lane report missing")?;
    assert!(matches!(
        dense_report.status,
        maestria_core::RetrievalLaneStatus::Succeeded
    ));
    assert!(!dense_report.candidates.is_empty());
    assert!(output.pack.chunks.is_empty());
    Ok(())
}

#[test]
fn active_mode_serves_dense_fusion() -> Result<(), Box<dyn std::error::Error>> {
    let record = maestria_core::HybridPromotionRecord::new(
        "eval-test".to_string(),
        "2026-07-16".to_string(),
    )
    .ok_or("promotion record must be non-empty")?;
    let output = search_with_policy(maestria_core::HybridExecutionPolicy::Active(record))?;
    assert_eq!(output.mode, maestria_core::RetrievalMode::Hybrid);
    assert_eq!(output.pack.chunks.len(), 1);
    Ok(())
}

#[test]
fn knowledge_search_trace_contains_deterministic_rewrites() -> Result<(), Box<dyn std::error::Error>>
{
    let (
        artifact_repo,
        chunk_repo,
        evidence_repo,
        blob_store,
        events,
        parser,
        search_index,
        _vector_index,
        _artifact_id,
        _chunk_id,
        _evidence_id,
    ) = seed_vector_fixture()?;
    let card_repo = maestria_ports::InMemoryCardRepository::new();
    let core = CoreServices::new(CorePorts {
        artifacts: &artifact_repo,
        chunks: &chunk_repo,
        cards: &card_repo,
        evidence: &evidence_repo,
        events: &events,
        parser: &parser,
        search_index: &search_index,
        blobs: &blob_store,
        vector_index: None,
        graph_index: None,
    });
    let plan = SearchPlan {
        query_id: QueryId::new(99),
        original_query: "find PR test".to_string(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::with_limits(1000, 30_000, 8, 1, 0)?,
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
        fingerprint: RetrievalModelFingerprint::new("maestria-core:deterministic-v1".to_string())?,
    };
    let mut invalid_plan = plan.clone();
    invalid_plan.original_query.clear();
    assert!(core.search_knowledge(invalid_plan).is_err());
    let outcome = core.search_knowledge(plan)?;
    let trace = outcome.trace_data.ok_or("trace data missing")?;
    assert_eq!(trace.original_query, "find PR test");
    assert!(
        trace.expansions.is_empty(),
        "initial-only plans must not claim context expansion"
    );
    assert!(
        trace
            .rewrites
            .iter()
            .any(|rewrite| rewrite.origin == maestria_domain::SearchRewriteOrigin::Original)
    );
    assert!(trace.rewrites.iter().any(|rewrite| {
        rewrite.origin == maestria_domain::SearchRewriteOrigin::Deterministic
            && rewrite.query.contains("Pull Request")
    }));
    Ok(())
}
