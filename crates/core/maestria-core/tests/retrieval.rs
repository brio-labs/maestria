use maestria_core::{
    CorePorts, CoreServices, GraphConfig, OpenChunkEvidenceInput, OpenEvidenceInput, SearchInput,
};
use maestria_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, Evidence, EvidenceId, EvidenceKind,
    IndexStatus, Relation, RelationEndpoint, RelationId, RelationKind, SourceSpan, StructureNodeId,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EvidenceRepository,
    FullTextIndex, GraphIndex, InMemoryArtifactRepository, InMemoryBlobStore,
    InMemoryCardRepository, InMemoryChunkRepository, InMemoryEventLog, InMemoryEvidenceRepository,
    InMemoryFullTextIndex, InMemoryGraphIndex, InMemoryParser, InMemoryVectorIndex, IndexedCard,
    IndexedChunk, VectorIndex, VectorSearchQuery,
};

/// Seed an artifact, chunks, evidence, cards, and full-text entries directly through
/// in-memory adapters, then wrap them in a `CoreServices` to exercise retrieval.
fn seed_and_build_services<'a>(ports: CorePorts<'a>) -> CoreServices<'a> {
    CoreServices::new(ports)
}

/// Run the retrieval assertions against a seeded `CoreServices`.
fn assert_directly_seeded_retrieval(
    core: &CoreServices,
    artifact_id: ArtifactId,
    card_id: CardId,
    chunk_id_0: ChunkId,
    chunk_id_1: ChunkId,
    evidence_id_0: EvidenceId,
    evidence_id_1: EvidenceId,
) -> Result<(), Box<dyn std::error::Error>> {
    // Search: "beta-token" should match chunk 1 but NOT cards (cards have different text).
    let search_result = core.search(SearchInput {
        query: "beta-token".to_string(),
        limit: 5,
    })?;
    assert_eq!(search_result.pack.cards.len(), 0);
    assert_eq!(search_result.pack.chunks.len(), 1);
    let hit = &search_result.pack.chunks[0];
    assert_eq!(hit.artifact.id, artifact_id);
    assert_eq!(hit.chunk.id, chunk_id_1);
    assert_eq!(hit.evidence.id, evidence_id_1);
    assert_eq!(hit.evidence.excerpt, "beta-token paragraph.");
    assert_eq!(search_result.pack.evidence_ids, &[evidence_id_1]);
    assert_eq!(search_result.pack.query, "beta-token");

    // Search: "card-title" should match the card.
    let card_result = core.search(SearchInput {
        query: "card-title".to_string(),
        limit: 5,
    })?;
    assert_eq!(card_result.pack.cards.len(), 1);
    assert_eq!(card_result.pack.chunks.len(), 0);
    assert!(card_result.pack.evidence_ids.is_empty());
    let card_hit = &card_result.pack.cards[0];
    assert_eq!(card_hit.artifact.id, artifact_id);
    assert_eq!(card_hit.card.id, card_id);

    // Open evidence by evidence id.
    let opened = core.open_evidence(OpenEvidenceInput {
        evidence_id: evidence_id_0,
    })?;
    assert_eq!(opened.artifact.id, artifact_id);
    assert_eq!(opened.evidence.id, evidence_id_0);
    assert_eq!(opened.evidence.excerpt, "alpha-token paragraph.");

    // Open evidence by chunk id.
    let chunk_opened = core.open_chunk_evidence(OpenChunkEvidenceInput {
        chunk_id: chunk_id_0,
    })?;
    assert_eq!(chunk_opened.evidence.id, evidence_id_0);

    Ok(())
}

struct GraphFixture {
    artifacts: InMemoryArtifactRepository,
    chunks: InMemoryChunkRepository,
    cards: InMemoryCardRepository,
    evidence: InMemoryEvidenceRepository,
    blobs: InMemoryBlobStore,
    events: InMemoryEventLog,
    parser: InMemoryParser,
    search: InMemoryFullTextIndex,
    graph: InMemoryGraphIndex,
    a: ArtifactId,
    b: ArtifactId,
    c: ArtifactId,
    d: ArtifactId,
    e: ArtifactId,
    f: ArtifactId,
    chunk_a: ChunkId,
    e_a: EvidenceId,
    e_b: EvidenceId,
    e_e: EvidenceId,
}

impl GraphFixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let artifacts = InMemoryArtifactRepository::new();
        let chunks = InMemoryChunkRepository::new();
        let evidence = InMemoryEvidenceRepository::new();
        let blobs = InMemoryBlobStore::new();
        let events = InMemoryEventLog::new();
        let parser = InMemoryParser::new();
        let search = InMemoryFullTextIndex::new();
        let graph = InMemoryGraphIndex::new();
        let blob_id = blobs.put(b"source".to_vec())?;
        let content_hash = maestria_core::content_hash(b"source");
        let seed =
            |id| seed_graph_artifact(&artifacts, &chunks, &evidence, blob_id, &content_hash, id);
        let (a, seed_chunk, e_a) = seed(1)?;
        let (b, _, e_b) = seed(2)?;
        let (c, _, _) = seed(3)?;
        let (d, _, _) = seed(4)?;
        let (e, _, e_e) = seed(5)?;
        let (f, _, _) = seed(6)?;
        search.index_chunks(vec![IndexedChunk {
            artifact_id: a,
            chunk_id: seed_chunk,
            text: "seed match".to_string(),
        }])?;
        let fixture = Self {
            artifacts,
            chunks,
            cards: InMemoryCardRepository::new(),
            evidence,
            blobs,
            events,
            parser,
            search,
            graph,
            a,
            b,
            c,
            d,
            e,
            chunk_a: seed_chunk,
            f,
            e_a,
            e_b,
            e_e,
        };
        fixture.insert_relations()?;
        Ok(fixture)
    }

    fn insert_relations(&self) -> Result<(), Box<dyn std::error::Error>> {
        let relation = |id, source, target, evidence_id, confidence_milli| Relation {
            id: RelationId::new(id),
            source: RelationEndpoint::Artifact(source),
            target: RelationEndpoint::Artifact(target),
            kind: RelationKind::Contains,
            evidence_id,
            confidence_milli,
            security: maestria_domain::SecurityMetadata::default(),
        };
        for relation in [
            relation(1, self.a, self.b, Some(self.e_e), 1000),
            relation(2, self.b, self.c, Some(self.e_b), 1000),
            relation(3, self.a, self.d, None, 1000),
            relation(4, self.c, self.e, Some(self.e_a), 1000),
            relation(5, self.e, self.a, Some(self.e_e), 1000),
            relation(6, self.a, self.f, Some(self.e_a), 500),
        ] {
            self.graph.insert_relation(relation)?;
        }
        Ok(())
    }

    fn core(&self, config: GraphConfig) -> CoreServices<'_> {
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
            graph_index: Some(&self.graph),
        })
        .with_graph_config(config)
    }

    fn core_without_graph(&self) -> CoreServices<'_> {
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
    }
}

fn seed_graph_artifact(
    artifacts: &InMemoryArtifactRepository,
    chunks: &InMemoryChunkRepository,
    evidence: &InMemoryEvidenceRepository,
    blob_id: maestria_domain::BlobId,
    content_hash: &str,
    id: u64,
) -> Result<(ArtifactId, ChunkId, EvidenceId), Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(id);
    let chunk_id = ChunkId::new(id + 100);
    let evidence_id = maestria_domain::evidence_id_for(artifact_id, 0);
    artifacts.put(Artifact {
        id: artifact_id,
        title: format!("art_{id}"),
        chunk_ids: [chunk_id].into(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: [evidence_id].into(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(content_hash.to_string()),
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
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
        text: "token".to_string(),
    })?;
    evidence.put(Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "file".to_string(),
            range: maestria_domain::ContentRange { start: 1, end: 1 },
            content_hash: content_hash.to_string(),
            snapshot: Some(blob_id),
        },
        excerpt: "source".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    })?;
    Ok((artifact_id, chunk_id, evidence_id))
}

#[test]
fn search_and_open_evidence_with_directly_seeded_artifact() -> Result<(), Box<dyn std::error::Error>>
{
    seed_with_status(
        IndexStatus::Indexed,
        |core, artifact_id, card_id, chunk_id_0, chunk_id_1, evidence_id_0, evidence_id_1| {
            assert_directly_seeded_retrieval(
                core,
                artifact_id,
                card_id,
                chunk_id_0,
                chunk_id_1,
                evidence_id_0,
                evidence_id_1,
            )
        },
    )
}

fn seed_file_evidence(
    evidence_repo: &InMemoryEvidenceRepository,
    artifact_id: ArtifactId,
    evidence_id_0: EvidenceId,
    evidence_id_1: EvidenceId,
    blob_id: maestria_domain::BlobId,
    source_text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let content_hash = maestria_core::content_hash(source_text.as_bytes());
    for (evidence_id, start, excerpt) in [
        (evidence_id_0, 1, "alpha-token paragraph."),
        (evidence_id_1, 3, "beta-token paragraph."),
    ] {
        evidence_repo.put(Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "notes/multi.md".to_string(),
                range: maestria_domain::ContentRange { start, end: start },
                content_hash: content_hash.clone(),
                snapshot: Some(blob_id),
            },
            excerpt: excerpt.to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: maestria_domain::SecurityMetadata::default(),
        })?;
    }
    Ok(())
}

/// Seed adapters with an artifact at the given `index_status`, then invoke
/// `f` with the assembled `CoreServices` and seeded ids.  All repositories
/// stay alive for the duration of the call so the borrowed `CoreServices`
/// reference remains valid.
#[allow(clippy::too_many_lines)]
fn seed_with_status(
    status: IndexStatus,
    f: impl FnOnce(
        &CoreServices,
        ArtifactId,
        CardId,
        ChunkId,
        ChunkId,
        EvidenceId,
        EvidenceId,
    ) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let artifact_repo = InMemoryArtifactRepository::new();
    let chunk_repo = InMemoryChunkRepository::new();
    let evidence_repo = InMemoryEvidenceRepository::new();
    let blob_store = InMemoryBlobStore::new();
    let search_index = InMemoryFullTextIndex::new();
    let events = InMemoryEventLog::new();
    let parser = InMemoryParser::new();
    let card_repo = InMemoryCardRepository::new();
    let artifact_id = ArtifactId::new(7);
    let chunk_id_0 = ChunkId::new(701);
    let chunk_id_1 = ChunkId::new(702);
    let evidence_id_0 = maestria_domain::evidence_id_for(artifact_id, 0);
    let card_id = CardId::new(700);
    let evidence_id_1 = maestria_domain::evidence_id_for(artifact_id, 1);

    let source_text = "alpha-token paragraph.\n\nbeta-token paragraph.\n";
    let blob_id = blob_store.put(source_text.as_bytes().to_vec())?;
    // Seed a card.
    let card = Card {
        id: card_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        title: "card-title summary".to_string(),
        body: "card body text".to_string(),
        claim_ids: Default::default(),
        security: maestria_domain::SecurityMetadata::default(),
    };
    card_repo.put(card.clone())?;

    artifact_repo.put(Artifact {
        id: artifact_id,
        title: "multi.md".to_string(),
        chunk_ids: [chunk_id_0, chunk_id_1].into(),
        card_ids: [card_id].into(),
        claim_ids: Default::default(),
        evidence_ids: [evidence_id_0, evidence_id_1].into(),
        index_status: status,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    })?;

    let chunk_0 = Chunk {
        id: chunk_id_0,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        order: 0,
        text: "alpha-token paragraph.".to_string(),
    };
    let chunk_1 = Chunk {
        id: chunk_id_1,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        order: 1,
        text: "beta-token paragraph.".to_string(),
    };
    chunk_repo.put(chunk_0.clone())?;
    chunk_repo.put(chunk_1.clone())?;

    seed_file_evidence(
        &evidence_repo,
        artifact_id,
        evidence_id_0,
        evidence_id_1,
        blob_id,
        source_text,
    )?;

    search_index.index_chunks(vec![
        IndexedChunk {
            artifact_id,
            chunk_id: chunk_id_0,
            text: chunk_0.text.clone(),
        },
        IndexedChunk {
            artifact_id,
            chunk_id: chunk_id_1,
            text: chunk_1.text.clone(),
        },
    ])?;

    // Index cards for full-text search.
    search_index.index_cards(vec![IndexedCard {
        artifact_id,
        card_id,
        title: card.title.clone(),
        body: card.body.clone(),
    }])?;

    let core = seed_and_build_services(CorePorts {
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

    f(
        &core,
        artifact_id,
        card_id,
        chunk_id_0,
        chunk_id_1,
        evidence_id_0,
        evidence_id_1,
    )
}

#[test]
fn search_excludes_pending_artifact() -> Result<(), Box<dyn std::error::Error>> {
    seed_with_status(
        IndexStatus::Pending,
        |core, _artifact_id, _card_id, _chunk_id_0, _chunk_id_1, _evidence_id_0, _evidence_id_1| {
            let search_result = core.search(SearchInput {
                query: "beta-token".to_string(),
                limit: 5,
            })?;

            assert!(
                search_result.pack.chunks.is_empty(),
                "pending artifact chunks must be excluded, got {}",
                search_result.pack.chunks.len()
            );
            assert!(
                search_result.pack.cards.is_empty(),
                "pending artifact cards must be excluded"
            );
            Ok(())
        },
    )
}

#[test]
fn search_excludes_unindexed_artifact() -> Result<(), Box<dyn std::error::Error>> {
    seed_with_status(
        IndexStatus::Unindexed,
        |core, _artifact_id, _card_id, _chunk_id_0, _chunk_id_1, _evidence_id_0, _evidence_id_1| {
            let search_result = core.search(SearchInput {
                query: "beta-token".to_string(),
                limit: 5,
            })?;

            assert!(
                search_result.pack.chunks.is_empty(),
                "unindexed artifact chunks must be excluded, got {}",
                search_result.pack.chunks.len()
            );
            assert!(
                search_result.pack.cards.is_empty(),
                "unindexed artifact cards must be excluded"
            );
            Ok(())
        },
    )
}

#[test]
fn open_evidence_rejects_pending_artifact() -> Result<(), Box<dyn std::error::Error>> {
    seed_with_status(
        IndexStatus::Pending,
        |core, _artifact_id, _card_id, _chunk_id_0, _chunk_id_1, evidence_id_0, _evidence_id_1| {
            let result = core.open_evidence(OpenEvidenceInput {
                evidence_id: evidence_id_0,
            });

            assert!(
                result.is_err(),
                "opening evidence for a pending artifact must fail"
            );

            let err = result.unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("not indexed"),
                "error must mention not indexed, got: {msg}"
            );
            Ok(())
        },
    )
}

#[test]
fn open_evidence_rejects_unindexed_artifact() -> Result<(), Box<dyn std::error::Error>> {
    seed_with_status(
        IndexStatus::Unindexed,
        |core, _artifact_id, _card_id, _chunk_id_0, _chunk_id_1, evidence_id_0, _evidence_id_1| {
            let result = core.open_evidence(OpenEvidenceInput {
                evidence_id: evidence_id_0,
            });

            assert!(
                result.is_err(),
                "opening evidence for an unindexed artifact must fail"
            );

            let err = result.unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("not indexed"),
                "error must mention not indexed, got: {msg}"
            );
            Ok(())
        },
    )
}

#[test]
fn open_chunk_evidence_rejects_pending_artifact() -> Result<(), Box<dyn std::error::Error>> {
    seed_with_status(
        IndexStatus::Pending,
        |core, _artifact_id, _card_id, chunk_id_0, _chunk_id_1, _evidence_id_0, _evidence_id_1| {
            let result = core.open_chunk_evidence(OpenChunkEvidenceInput {
                chunk_id: chunk_id_0,
            });

            assert!(
                result.is_err(),
                "opening chunk evidence for a pending artifact must fail"
            );

            let err = result.unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("not indexed"),
                "error must mention not indexed, got: {msg}"
            );
            Ok(())
        },
    )
}

#[test]
fn terminal_success_with_indexed_artifact() -> Result<(), Box<dyn std::error::Error>> {
    seed_with_status(
        IndexStatus::Indexed,
        |core, artifact_id, card_id, chunk_id_0, chunk_id_1, evidence_id_0, evidence_id_1| {
            assert_directly_seeded_retrieval(
                core,
                artifact_id,
                card_id,
                chunk_id_0,
                chunk_id_1,
                evidence_id_0,
                evidence_id_1,
            )
        },
    )
}

#[test]
#[allow(clippy::too_many_lines)]
fn vector_search_returns_grounded_nonliteral_match() -> Result<(), Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(800);
    let chunk_id = ChunkId::new(801);
    let evidence_id = maestria_domain::evidence_id_for(artifact_id, 0);
    let source = "literal source text\n";
    let artifact_repo = InMemoryArtifactRepository::new();
    let chunk_repo = InMemoryChunkRepository::new();
    let card_repo = InMemoryCardRepository::new();
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
            provider_id: "test-provider".to_string(),
            model: "test-model".to_string(),
            model_version: "test-v1".to_string(),
        },
    }])?;
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
    });

    let pack = core
        .search_with_vector(
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
            },
        )?
        .pack;
    assert_eq!(pack.chunks.len(), 1);
    assert_eq!(pack.chunks[0].chunk.id, chunk_id);
    assert_eq!(pack.chunks[0].evidence.id, evidence_id);
    assert_eq!(pack.evidence_ids, vec![evidence_id]);
    Ok(())
}

#[test]
fn test_graph_retrieval_integration() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GraphFixture::new()?;
    let pack = fixture
        .core(GraphConfig {
            max_depth: 2,
            max_results: 10,
        })
        .search(SearchInput {
            query: "seed match".to_string(),
            limit: 10,
        })?
        .pack;

    let chunk_artifacts: Vec<_> = pack.chunks.iter().map(|hit| hit.artifact.id).collect();
    assert_eq!(chunk_artifacts.len(), 5);
    assert!(chunk_artifacts.contains(&fixture.a));
    assert!(chunk_artifacts.contains(&fixture.b));
    assert!(chunk_artifacts.contains(&fixture.c));
    assert!(!chunk_artifacts.contains(&fixture.d));
    assert!(chunk_artifacts.contains(&fixture.e));
    assert!(chunk_artifacts.contains(&fixture.f));
    assert!(pack.evidence_ids.contains(&fixture.e_e));
    let score_b = pack
        .chunks
        .iter()
        .find(|hit| hit.artifact.id == fixture.b)
        .map(|hit| hit.score)
        .ok_or("missing B graph hit")?;
    let score_f = pack
        .chunks
        .iter()
        .find(|hit| hit.artifact.id == fixture.f)
        .map(|hit| hit.score)
        .ok_or("missing F graph hit")?;
    assert!(score_b > score_f);

    let capped = fixture
        .core(GraphConfig {
            max_depth: 3,
            max_results: 2,
        })
        .search(SearchInput {
            query: "seed match".to_string(),
            limit: 10,
        })?
        .pack;
    assert_eq!(capped.chunks.len(), 3);

    let fallback = fixture
        .core_without_graph()
        .search(SearchInput {
            query: "seed match".to_string(),
            limit: 10,
        })?
        .pack;
    assert_eq!(fallback.chunks.len(), 1);
    assert_eq!(fallback.chunks[0].artifact.id, fixture.a);
    Ok(())
}

#[test]
fn retrieval_policy_filters_before_scoring() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GraphFixture::new()?;
    let policy =
        RetrievalSecurityPolicy::new().require_trust_zone(maestria_domain::TrustZone::System);
    let pack = fixture
        .core_without_graph()
        .with_retrieval_policy(policy)
        .search(SearchInput {
            query: "seed match".to_string(),
            limit: 10,
        })?
        .pack;

    assert!(pack.cards.is_empty());
    assert!(pack.chunks.is_empty());
    assert!(pack.evidence_ids.is_empty());
    let restricted_core = fixture.core_without_graph().with_retrieval_policy(
        RetrievalSecurityPolicy::new().require_trust_zone(maestria_domain::TrustZone::System),
    );
    assert!(
        restricted_core
            .open_evidence(OpenEvidenceInput {
                evidence_id: fixture.e_a,
            })
            .is_err()
    );
    assert!(
        restricted_core
            .open_chunk_evidence(OpenChunkEvidenceInput {
                chunk_id: fixture.chunk_a,
            })
            .is_err()
    );
    Ok(())
}
