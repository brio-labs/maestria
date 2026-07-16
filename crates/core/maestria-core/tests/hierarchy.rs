use maestria_core::{CorePorts, CoreServices, GraphConfig, SearchInput};
use maestria_domain::{
    Artifact, ArtifactId, ContentHash, ContentRange, DomainEvent, DomainEventEnvelope, EventId,
    Evidence, EvidenceKind, IndexStatus, LogicalTick, SequenceNumber, SourceSpan, StructureNode,
    StructureNodeId, StructureNodeType,
};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EventLog, EvidenceRepository, FullTextIndex,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository, InMemoryChunkRepository,
    InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryParser,
    IndexedChunk,
};

const ARTIFACT_ID: ArtifactId = ArtifactId::new(1);
const SOURCE: &[u8] = b"seed match\nchild context\nsibling context\ngrandchild context";

struct Fixture {
    artifacts: InMemoryArtifactRepository,
    chunks: InMemoryChunkRepository,
    cards: InMemoryCardRepository,
    evidence: InMemoryEvidenceRepository,
    blobs: InMemoryBlobStore,
    events: InMemoryEventLog,
    parser: InMemoryParser,
    search: InMemoryFullTextIndex,
}

impl Fixture {
    fn core(&self) -> CoreServices<'_> {
        CoreServices::new(CorePorts {
            artifacts: &self.artifacts,
            chunks: &self.chunks,
            cards: &self.cards,
            evidence: &self.evidence,
            events: &self.events,
            parser: &self.parser,
            search_index: &self.search,
            blobs: &self.blobs,
            vector_index: None,
            graph_index: None,
        })
        .with_graph_config(GraphConfig {
            max_depth: 2,
            max_results: 10,
        })
    }
}

fn setup() -> Result<Fixture, Box<dyn std::error::Error>> {
    let artifacts = InMemoryArtifactRepository::new();
    let chunks = InMemoryChunkRepository::new();
    let cards = InMemoryCardRepository::new();
    let evidence = InMemoryEvidenceRepository::new();
    let blobs = InMemoryBlobStore::new();
    let events = InMemoryEventLog::new();
    let parser = InMemoryParser::new();
    let search = InMemoryFullTextIndex::new();
    let blob_id = blobs.put(SOURCE.to_vec())?;
    let content_hash = maestria_core::content_hash(SOURCE);

    artifacts.put(Artifact {
        id: ARTIFACT_ID,
        title: "hierarchy.md".to_string(),
        chunk_ids: Default::default(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(content_hash.clone()),
        parse_status: None,
        security: Default::default(),
    })?;
    capture_tree(&events, &content_hash)?;
    seed_chunks(&chunks, &evidence, &search, blob_id, &content_hash)?;
    Ok(Fixture {
        artifacts,
        chunks,
        cards,
        evidence,
        blobs,
        events,
        parser,
        search,
    })
}

fn capture_tree(
    events: &InMemoryEventLog,
    content_hash: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = StructureNodeId::new(1);
    let nodes = vec![
        node(root, None, None, 1, 4, vec![]),
        node(
            StructureNodeId::new(2),
            Some(root),
            Some(StructureNodeId::new(3)),
            2,
            2,
            vec!["Child".to_string()],
        ),
        node(
            StructureNodeId::new(3),
            Some(root),
            None,
            3,
            3,
            vec!["Sibling".to_string()],
        ),
        node(
            StructureNodeId::new(4),
            Some(StructureNodeId::new(2)),
            None,
            4,
            4,
            vec!["Child".to_string(), "Grandchild".to_string()],
        ),
    ];
    events.append(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::DocumentTreeCaptured {
            artifact_id: ARTIFACT_ID,
            artifact_version_id: maestria_domain::ArtifactVersionId::new(1),
            content_hash: ContentHash::new(content_hash.to_string())?,
            root_id: root,
            nodes,
        },
    })?;
    Ok(())
}

fn node(
    id: StructureNodeId,
    parent_id: Option<StructureNodeId>,
    sibling_id: Option<StructureNodeId>,
    start: usize,
    end: usize,
    section_path: Vec<String>,
) -> StructureNode {
    StructureNode {
        id,
        parent_id,
        sibling_id,
        node_type: if parent_id.is_none() {
            StructureNodeType::Document
        } else {
            StructureNodeType::Section
        },
        source_range: ContentRange { start, end },
        page: None,
        section_path,
        parser_generation: "test".to_string(),
        schema_generation: "test".to_string(),
        language: None,
    }
}

fn seed_chunks(
    chunks: &InMemoryChunkRepository,
    evidence: &InMemoryEvidenceRepository,
    search: &InMemoryFullTextIndex,
    blob_id: maestria_domain::BlobId,
    content_hash: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for (order, node_id, text) in [
        (0_u32, 1_u64, "seed match"),
        (1, 2, "child context"),
        (2, 3, "sibling context"),
        (3, 4, "grandchild context"),
    ] {
        let chunk_id = maestria_domain::ChunkId::new(11 + u64::from(order));
        chunks.put(maestria_domain::Chunk {
            id: chunk_id,
            artifact_id: ARTIFACT_ID,
            node_id: StructureNodeId::new(node_id),
            source_span: SourceSpan::TextSpan {
                start_line: order as usize + 1,
                end_line: order as usize + 1,
            },
            representations: vec![],
            order,
            text: text.to_string(),
        })?;
        evidence.put(Evidence {
            id: maestria_domain::evidence_id_for(ARTIFACT_ID, order),
            artifact_id: ARTIFACT_ID,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "hierarchy.md".to_string(),
                range: ContentRange {
                    start: order as usize + 1,
                    end: order as usize + 1,
                },
                content_hash: content_hash.to_string(),
                snapshot: Some(blob_id),
            },
            excerpt: text.to_string(),
            observed_at: LogicalTick::new(1),
            security: Default::default(),
        })?;
        if order == 0 {
            search.index_chunks(vec![IndexedChunk {
                artifact_id: ARTIFACT_ID,
                chunk_id,
                text: text.to_string(),
            }])?;
        }
    }
    Ok(())
}

#[test]
fn hierarchy_expands_children_and_siblings_with_query_adaptive_depth()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = setup()?;
    let core = fixture.core();
    let precise = core.search(SearchInput {
        query: "\"seed match\"".to_string(),
        limit: 10,
    })?;
    assert_eq!(precise.pack.chunks.len(), 3);
    assert_eq!(
        precise
            .pack
            .chunks
            .iter()
            .map(|hit| hit.chunk.id)
            .collect::<Vec<_>>(),
        vec![
            maestria_domain::ChunkId::new(11),
            maestria_domain::ChunkId::new(12),
            maestria_domain::ChunkId::new(13),
        ]
    );
    let limited = core.search(SearchInput {
        query: "\"seed match\"".to_string(),
        limit: 1,
    })?;
    assert_eq!(limited.pack.chunks.len(), 1);

    let broad = core.search(SearchInput {
        query: "seed match".to_string(),
        limit: 10,
    })?;
    assert_eq!(broad.pack.chunks.len(), 4);
    let empty = core.search(SearchInput {
        query: "broad query with context".to_string(),
        limit: 0,
    })?;
    assert!(empty.pack.cards.is_empty());
    assert!(empty.pack.chunks.is_empty());
    assert!(empty.pack.evidence_ids.is_empty());
    Ok(())
}
