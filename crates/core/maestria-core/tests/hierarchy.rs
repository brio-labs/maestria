use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use maestria_domain::{
    Artifact, ArtifactId, ArtifactVersionId, Chunk, ChunkId, ContentHash, ContentRange,
    CorpusScope, CorpusSnapshotId, Evidence, EvidenceCandidate, EvidenceId, EvidenceKind,
    EvidenceSpan, FreshnessStatus, IndexGenerationId, IndexStatus, LineRange, LogicalTick,
    Relation, RelationEndpoint, RelationId, RelationKind, RetrievalModelFingerprint,
    RetrievalScoreSet, SearchOutcome, SearchStatus, SnapshotRef, SourceLocation, SourceSpan,
    StructureNodeId, TrustLabel,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, FullTextIndex, GraphIndex,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
    InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryGraphIndex, IndexedChunk, PortError,
};
use maestria_retrieval::{
    ContextExpander, RetrievalEngine, SearchPlannerContext,
    adapters::{
        EvidenceOutcomeEvaluator, HierarchyGraphExpander, HierarchyGraphExpanderParts,
        LexicalChunkRetriever, LexicalChunkRetrieverParts,
    },
    types::{ExpansionPolicy, RankedCandidate},
};

const ROOT: ArtifactId = ArtifactId::new(1);
const CHILD: ArtifactId = ArtifactId::new(2);
const SIBLING: ArtifactId = ArtifactId::new(3);
const GRANDCHILD: ArtifactId = ArtifactId::new(4);

#[derive(Clone)]
struct CountingGraphIndex {
    inner: Arc<InMemoryGraphIndex>,
    lookups: Arc<AtomicUsize>,
}

impl CountingGraphIndex {
    fn new() -> Self {
        Self {
            inner: Arc::new(InMemoryGraphIndex::new()),
            lookups: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn lookup_count(&self) -> usize {
        self.lookups.load(Ordering::Relaxed)
    }
}

impl GraphIndex for CountingGraphIndex {
    fn insert_relation(&self, relation: Relation) -> Result<(), PortError> {
        self.inner.insert_relation(relation)
    }

    fn get_relations_for(&self, endpoint: RelationEndpoint) -> Result<Vec<Relation>, PortError> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        self.inner.get_relations_for(endpoint)
    }

    fn delete_relations(&self, relation_ids: &[RelationId]) -> Result<(), PortError> {
        self.inner.delete_relations(relation_ids)
    }

    fn clear(&self) -> Result<(), PortError> {
        self.inner.clear()
    }
}

#[derive(Clone)]
struct CountingEvidenceRepository {
    inner: Arc<InMemoryEvidenceRepository>,
    gets: Arc<AtomicUsize>,
}

impl CountingEvidenceRepository {
    fn new(inner: Arc<InMemoryEvidenceRepository>) -> Self {
        Self {
            inner,
            gets: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn get_count(&self) -> usize {
        self.gets.load(Ordering::Relaxed)
    }
}

impl EvidenceRepository for CountingEvidenceRepository {
    fn get(&self, evidence_id: EvidenceId) -> Result<Option<Evidence>, PortError> {
        self.gets.fetch_add(1, Ordering::Relaxed);
        self.inner.get(evidence_id)
    }

    fn put(&self, evidence: Evidence) -> Result<(), PortError> {
        self.inner.put(evidence)
    }

    fn replace(&self, evidence: Evidence) -> Result<(), PortError> {
        self.inner.replace(evidence)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError> {
        self.inner.list_for_artifact(artifact_id)
    }
}

fn ranked_seed(
    artifact_id: ArtifactId,
    evidence_id: EvidenceId,
) -> Result<RankedCandidate, Box<dyn std::error::Error>> {
    Ok(RankedCandidate {
        rank: 0,
        candidate: EvidenceCandidate {
            evidence_id,
            artifact_version: ArtifactVersionId::new(artifact_id.value()),
            source_span: EvidenceSpan::new(
                Some(StructureNodeId::new(0)),
                SourceLocation::File {
                    path: format!("hierarchy-{artifact_id}.md"),
                    start_line: 1,
                    end_line: 1,
                },
                ContentRange { start: 1, end: 1 },
            )?,
            scores: RetrievalScoreSet::empty(),
            trust: TrustLabel::Verified,
            freshness: FreshnessStatus::UpToDate,
            duplicate_cluster: None,
            reasons: Vec::new(),
            coverage_keys: Vec::new(),
        },
    })
}
struct Fixture {
    artifacts: Arc<InMemoryArtifactRepository>,
    chunks: Arc<InMemoryChunkRepository>,
    evidence: Arc<InMemoryEvidenceRepository>,
    blobs: Arc<InMemoryBlobStore>,
    graph_index: Arc<InMemoryGraphIndex>,
    search_index: Arc<InMemoryFullTextIndex>,
}
struct SeedRepositories<'a> {
    artifacts: &'a InMemoryArtifactRepository,
    chunks: &'a InMemoryChunkRepository,
    evidence: &'a InMemoryEvidenceRepository,
    blobs: &'a InMemoryBlobStore,
    search_index: &'a InMemoryFullTextIndex,
}

fn setup() -> Result<(Fixture, ArtifactId, ArtifactId, ArtifactId), Box<dyn std::error::Error>> {
    let artifacts = Arc::new(InMemoryArtifactRepository::new());
    let chunks = Arc::new(InMemoryChunkRepository::new());
    let evidence = Arc::new(InMemoryEvidenceRepository::new());
    let blobs = Arc::new(InMemoryBlobStore::new());
    let graph_index = Arc::new(InMemoryGraphIndex::new());
    let search_index = Arc::new(InMemoryFullTextIndex::new());
    let seed_repositories = SeedRepositories {
        artifacts: &artifacts,
        chunks: &chunks,
        evidence: &evidence,
        blobs: &blobs,
        search_index: &search_index,
    };

    let root_chunk_id = ChunkId::new(11);
    let child_chunk_id = ChunkId::new(12);
    let sibling_chunk_id = ChunkId::new(13);
    let grandchild_chunk_id = ChunkId::new(14);

    let root_evidence = seed_artifact(&seed_repositories, ROOT, root_chunk_id, "\"seed match\"")?;
    seed_artifact(
        &seed_repositories,
        CHILD,
        child_chunk_id,
        "child seed context",
    )?;
    seed_artifact(
        &seed_repositories,
        SIBLING,
        sibling_chunk_id,
        "sibling seed context",
    )?;
    seed_artifact(
        &seed_repositories,
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
    repositories: &SeedRepositories<'_>,
    artifact_id: ArtifactId,
    chunk_id: ChunkId,
    text: &str,
) -> Result<EvidenceId, Box<dyn std::error::Error>> {
    let snapshot_id = repositories.blobs.put(text.as_bytes().to_vec())?;
    repositories.artifacts.put(Artifact {
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
    repositories.chunks.put(Chunk {
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
    repositories.evidence.put(Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: format!("hierarchy-{artifact_id}.md"),
            range: LineRange::new(1, 1)?,
            snapshot: SnapshotRef::new(
                snapshot_id,
                ContentHash::new(maestria_core::content_hash(text.as_bytes()))?,
            ),
        },
        excerpt: text.to_string(),
        observed_at: LogicalTick::new(1),
        security: Default::default(),
    })?;
    repositories.search_index.index_chunks(vec![IndexedChunk {
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
        context.primary_generation,
    ));
    let expander = Arc::new(HierarchyGraphExpander::new(HierarchyGraphExpanderParts {
        graph: fixture.graph_index.clone(),
        artifacts: fixture.artifacts.clone(),
        chunks: fixture.chunks.clone(),
        evidence: fixture.evidence.clone(),
        blobs: fixture.blobs.clone(),
    }));
    RetrievalEngine::new(
        vec![lexical],
        Arc::new(EvidenceOutcomeEvaluator::new(fixture.evidence.clone())),
        maestria_governance::RetrievalSecurityPolicy::default(),
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

#[test]
fn high_degree_graph_caps_relation_and_evidence_lookups() -> Result<(), Box<dyn std::error::Error>>
{
    let artifacts = Arc::new(InMemoryArtifactRepository::new());
    let chunks = Arc::new(InMemoryChunkRepository::new());
    let evidence = Arc::new(InMemoryEvidenceRepository::new());
    let blobs = Arc::new(InMemoryBlobStore::new());
    let search_index = InMemoryFullTextIndex::new();
    let graph = Arc::new(CountingGraphIndex::new());
    let seed_repositories = SeedRepositories {
        artifacts: &artifacts,
        chunks: &chunks,
        evidence: &evidence,
        blobs: &blobs,
        search_index: &search_index,
    };

    let root_evidence = seed_artifact(
        &seed_repositories,
        ROOT,
        ChunkId::new(11),
        "high-degree seed",
    )?;
    let child_evidence = seed_artifact(
        &seed_repositories,
        CHILD,
        ChunkId::new(12),
        "first related context",
    )?;

    for relation_index in 0..128_u64 {
        let target = if relation_index == 0 {
            CHILD
        } else {
            ArtifactId::new(1_000 + relation_index)
        };
        graph.insert_relation(Relation {
            id: RelationId::new(relation_index + 1),
            source: RelationEndpoint::Artifact(ROOT),
            kind: RelationKind::Contains,
            target: RelationEndpoint::Artifact(target),
            evidence_id: Some(root_evidence),
            confidence_milli: 1_000,
            security: Default::default(),
        })?;
    }

    let counted_evidence = Arc::new(CountingEvidenceRepository::new(evidence.clone()));
    let expander = HierarchyGraphExpander::new(HierarchyGraphExpanderParts {
        graph: graph.clone(),
        artifacts,
        chunks,
        evidence: counted_evidence.clone(),
        blobs,
    });
    let seed = ranked_seed(ROOT, root_evidence)?;
    let expanded = expander.expand(
        std::slice::from_ref(&seed),
        &ExpansionPolicy {
            max_results: 2,
            max_depth: 1,
            selected_seeds: vec![seed.candidate.clone()],
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            authorization: RetrievalSecurityPolicy::default()
                .authorization_context(&CorpusScope::Global)?,
        },
    )?;

    assert_eq!(
        expanded
            .iter()
            .map(|candidate| candidate.evidence_id)
            .collect::<Vec<_>>(),
        vec![root_evidence, child_evidence]
    );
    assert_eq!(expanded.len(), 2);
    assert_eq!(graph.lookup_count(), 1);
    assert_eq!(counted_evidence.get_count(), 2);
    Ok(())
}
