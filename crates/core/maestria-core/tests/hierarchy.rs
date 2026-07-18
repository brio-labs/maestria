use std::sync::Arc;

use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, ContentRange, CorpusSnapshotId, Evidence, EvidenceId,
    EvidenceKind, IndexGenerationId, IndexStatus, LogicalTick, Relation, RelationEndpoint,
    RelationId, RelationKind, RetrievalModelFingerprint, SearchOutcome, SearchStatus, SourceSpan,
    StructureNodeId,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, ChunkRepository, EvidenceRepository, FullTextIndex, GraphIndex,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
    InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryGraphIndex, IndexedChunk,
};
use maestria_retrieval::{
    RetrievalEngine, SearchPlannerContext,
    adapters::{
        EvidenceOutcomeEvaluator, HierarchyGraphExpander, HierarchyGraphExpanderParts,
        LexicalChunkRetriever, LexicalChunkRetrieverParts,
    },
};

const ROOT: ArtifactId = ArtifactId::new(1);
const CHILD: ArtifactId = ArtifactId::new(2);
const SIBLING: ArtifactId = ArtifactId::new(3);
const GRANDCHILD: ArtifactId = ArtifactId::new(4);

struct Fixture {
    artifacts: Arc<InMemoryArtifactRepository>,
    chunks: Arc<InMemoryChunkRepository>,
    evidence: Arc<InMemoryEvidenceRepository>,
    blobs: Arc<InMemoryBlobStore>,
    graph_index: Arc<InMemoryGraphIndex>,
    search_index: Arc<InMemoryFullTextIndex>,
}

fn setup() -> Result<(Fixture, ArtifactId, ArtifactId, ArtifactId), Box<dyn std::error::Error>> {
    let artifacts = Arc::new(InMemoryArtifactRepository::new());
    let chunks = Arc::new(InMemoryChunkRepository::new());
    let evidence = Arc::new(InMemoryEvidenceRepository::new());
    let blobs = Arc::new(InMemoryBlobStore::new());
    let graph_index = Arc::new(InMemoryGraphIndex::new());
    let search_index = Arc::new(InMemoryFullTextIndex::new());

    let root_chunk_id = ChunkId::new(11);
    let child_chunk_id = ChunkId::new(12);
    let sibling_chunk_id = ChunkId::new(13);
    let grandchild_chunk_id = ChunkId::new(14);

    let root_evidence = seed_artifact(
        &artifacts,
        &chunks,
        &evidence,
        &search_index,
        ROOT,
        root_chunk_id,
        "\"seed match\"",
    )?;
    seed_artifact(
        &artifacts,
        &chunks,
        &evidence,
        &search_index,
        CHILD,
        child_chunk_id,
        "child seed context",
    )?;
    seed_artifact(
        &artifacts,
        &chunks,
        &evidence,
        &search_index,
        SIBLING,
        sibling_chunk_id,
        "sibling seed context",
    )?;
    seed_artifact(
        &artifacts,
        &chunks,
        &evidence,
        &search_index,
        GRANDCHILD,
        grandchild_chunk_id,
        "grandchild seed context",
    )?;

    graph_index.insert_relation(Relation {
        id: RelationId::new(1),
        source: RelationEndpoint::Artifact(ROOT),
        kind: RelationKind::Contains,
        target: RelationEndpoint::Artifact(CHILD),
        evidence_id: Some(root_evidence),
        confidence_milli: 1000,
        security: Default::default(),
    })?;
    graph_index.insert_relation(Relation {
        id: RelationId::new(2),
        source: RelationEndpoint::Artifact(ROOT),
        kind: RelationKind::Contains,
        target: RelationEndpoint::Artifact(SIBLING),
        evidence_id: Some(root_evidence),
        confidence_milli: 1000,
        security: Default::default(),
    })?;
    graph_index.insert_relation(Relation {
        id: RelationId::new(3),
        source: RelationEndpoint::Artifact(CHILD),
        kind: RelationKind::Contains,
        target: RelationEndpoint::Artifact(GRANDCHILD),
        evidence_id: Some(root_evidence),
        confidence_milli: 1000,
        security: Default::default(),
    })?;

    Ok((
        Fixture {
            artifacts,
            chunks,
            evidence,
            blobs,
            graph_index,
            search_index,
        },
        ROOT,
        CHILD,
        SIBLING,
    ))
}

fn seed_artifact(
    artifacts: &InMemoryArtifactRepository,
    chunks: &InMemoryChunkRepository,
    evidence: &InMemoryEvidenceRepository,
    search_index: &InMemoryFullTextIndex,
    artifact_id: ArtifactId,
    chunk_id: ChunkId,
    text: &str,
) -> Result<EvidenceId, Box<dyn std::error::Error>> {
    artifacts.put(Artifact {
        id: artifact_id,
        title: format!("hierarchy-{artifact_id}.md"),
        chunk_ids: [chunk_id].into(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(maestria_core::content_hash(text.as_bytes())),
        parse_status: None,
        security: Default::default(),
    })?;
    chunks.put(Chunk {
        id: chunk_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        order: 0,
        text: text.to_string(),
    })?;
    let evidence_id = maestria_domain::evidence_id_for(artifact_id, 0);
    evidence.put(Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: format!("hierarchy-{artifact_id}.md"),
            range: ContentRange { start: 1, end: 1 },
            content_hash: maestria_core::content_hash(text.as_bytes()),
            snapshot: None,
        },
        excerpt: text.to_string(),
        observed_at: LogicalTick::new(1),
        security: Default::default(),
    })?;
    search_index.index_chunks(vec![IndexedChunk {
        artifact_id,
        chunk_id,
        text: text.to_string(),
    }])?;
    Ok(evidence_id)
}

fn with_engine(fixture: &Fixture, context: &SearchPlannerContext) -> RetrievalEngine {
    let lexical = Arc::new(LexicalChunkRetriever::new(
        LexicalChunkRetrieverParts {
            index: fixture.search_index.clone(),
            artifacts: fixture.artifacts.clone(),
            chunks: fixture.chunks.clone(),
            evidence: fixture.evidence.clone(),
            blobs: fixture.blobs.clone(),
        },
        RetrievalSecurityPolicy::default(),
        context.primary_generation,
    ));
    let expander = Arc::new(HierarchyGraphExpander::new(
        HierarchyGraphExpanderParts {
            graph: fixture.graph_index.clone(),
            artifacts: fixture.artifacts.clone(),
            chunks: fixture.chunks.clone(),
            evidence: fixture.evidence.clone(),
            blobs: fixture.blobs.clone(),
        },
        RetrievalSecurityPolicy::default(),
    ));
    RetrievalEngine::new(
        vec![lexical],
        Arc::new(EvidenceOutcomeEvaluator::new(fixture.evidence.clone())),
    )
    .with_expander(expander)
}

fn execute_search(
    engine: &RetrievalEngine,
    context: &SearchPlannerContext,
    query: &str,
    limit: usize,
) -> Result<SearchOutcome, Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut plan = engine.plan(query.to_string(), limit, context)?;
    if query.starts_with('"') && query.ends_with('"') && limit > 1 {
        plan.stop_conditions.max_results = 3;
    }
    plan.evidence_requirements.minimum_sources = 3;
    runtime.block_on(engine.search(&plan)).map_err(|error| {
        Box::<dyn std::error::Error>::from(std::io::Error::other(error.to_string()))
    })
}

#[test]
fn hierarchy_expands_children_and_siblings_with_query_adaptive_depth()
-> Result<(), Box<dyn std::error::Error>> {
    let (fixture, root_artifact, child_artifact, sibling_artifact) = setup()?;
    let context = SearchPlannerContext {
        corpus_snapshot: CorpusSnapshotId::new(1),
        primary_generation: IndexGenerationId::new(1),
        fingerprint: RetrievalModelFingerprint::new("maestria-core-hierarchy".to_string())?,
    };

    let engine = with_engine(&fixture, &context);

    let precise = execute_search(&engine, &context, "\"seed match\"", 10)?;
    assert_eq!(precise.status, SearchStatus::Answerable);
    assert_eq!(
        precise
            .evidence
            .iter()
            .map(|candidate| candidate.evidence_id)
            .collect::<Vec<_>>(),
        vec![
            maestria_domain::evidence_id_for(root_artifact, 0),
            maestria_domain::evidence_id_for(child_artifact, 0),
            maestria_domain::evidence_id_for(sibling_artifact, 0),
        ]
    );

    let limited = execute_search(&engine, &context, "\"seed match\"", 1)?;
    assert_eq!(limited.status, SearchStatus::AnswerableWithWarnings);
    assert_eq!(limited.evidence.len(), 1);

    let broad = execute_search(&engine, &context, "seed", 10)?;
    assert_eq!(broad.status, SearchStatus::Answerable);
    assert_eq!(broad.evidence.len(), 4);

    let empty = execute_search(&engine, &context, "unmatched query", 0)?;
    assert_eq!(empty.evidence.len(), 0);
    assert_eq!(empty.coverage.percent_covered, 0);
    Ok(())
}
