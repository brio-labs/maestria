use maestria_core::{
    CoreError, CorePorts, CoreServices, OpenChunkEvidenceInput, OpenEvidenceInput,
};
use maestria_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, Evidence, EvidenceId, EvidenceKind,
    IndexStatus, SourceSpan, StructureNodeId,
};
use maestria_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EvidenceRepository, FullTextIndex,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository, InMemoryChunkRepository,
    InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryParser,
    IndexedCard, IndexedChunk,
};

type SeedIds = (ArtifactId, CardId, ChunkId, ChunkId, EvidenceId, EvidenceId);

fn seed_records(
    status: IndexStatus,
    artifacts: &InMemoryArtifactRepository,
    chunks: &InMemoryChunkRepository,
    cards: &InMemoryCardRepository,
    evidence: &InMemoryEvidenceRepository,
    ids: SeedIds,
) -> Result<(), Box<dyn std::error::Error>> {
    let (artifact_id, card_id, chunk_id_0, chunk_id_1, evidence_id_0, evidence_id_1) = ids;
    artifacts.put(Artifact {
        id: artifact_id,
        title: "multi.md".to_string(),
        chunk_ids: [chunk_id_0, chunk_id_1].into(),
        card_ids: [card_id].into(),
        claim_ids: Default::default(),
        evidence_ids: [evidence_id_0, evidence_id_1].into(),
        index_status: status,
        content_hash: None,
        parse_status: None,
        security: Default::default(),
    })?;
    cards.put(Card {
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
        security: Default::default(),
    })?;
    for (id, order, text) in [
        (chunk_id_0, 0, "alpha-token paragraph."),
        (chunk_id_1, 1, "beta-token paragraph."),
    ] {
        chunks.put(Chunk {
            id,
            artifact_id,
            node_id: StructureNodeId::new(0),
            source_span: SourceSpan::TextSpan {
                start_line: order + 1,
                end_line: order + 1,
            },
            representations: vec![],
            order: order as u32,
            text: text.to_string(),
        })?;
    }
    for (id, order, excerpt) in [
        (evidence_id_0, 1, "alpha-token paragraph."),
        (evidence_id_1, 2, "beta-token paragraph."),
    ] {
        evidence.put(Evidence {
            id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "multi.md".to_string(),
                range: maestria_domain::ContentRange {
                    start: order,
                    end: order,
                },
                content_hash: maestria_core::content_hash(
                    b"alpha-token paragraph.\nbeta-token paragraph.",
                ),
                snapshot: None,
            },
            excerpt: excerpt.to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: Default::default(),
        })?;
    }
    Ok(())
}

fn seed_indexes(
    search_index: &InMemoryFullTextIndex,
    ids: SeedIds,
) -> Result<(), Box<dyn std::error::Error>> {
    let (artifact_id, card_id, chunk_id_0, chunk_id_1, _, _) = ids;
    search_index.index_chunks(vec![
        IndexedChunk {
            artifact_id,
            chunk_id: chunk_id_0,
            text: "alpha-token paragraph.".to_string(),
        },
        IndexedChunk {
            artifact_id,
            chunk_id: chunk_id_1,
            text: "beta-token paragraph.".to_string(),
        },
    ])?;
    search_index.index_cards(vec![IndexedCard {
        artifact_id,
        card_id,
        title: "card-title summary".to_string(),
        body: "card body text".to_string(),
    }])?;
    Ok(())
}

fn seed_fixture(
    status: IndexStatus,
    artifacts: &InMemoryArtifactRepository,
    chunks: &InMemoryChunkRepository,
    cards: &InMemoryCardRepository,
    evidence: &InMemoryEvidenceRepository,
    search_index: &InMemoryFullTextIndex,
) -> Result<SeedIds, Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(7);
    let card_id = CardId::new(700);
    let chunk_id_0 = ChunkId::new(701);
    let chunk_id_1 = ChunkId::new(702);
    let ids = (
        artifact_id,
        card_id,
        chunk_id_0,
        chunk_id_1,
        maestria_domain::evidence_id_for(artifact_id, 0),
        maestria_domain::evidence_id_for(artifact_id, 1),
    );
    seed_records(status, artifacts, chunks, cards, evidence, ids)?;
    seed_indexes(search_index, ids)?;
    Ok(ids)
}

fn with_seed(
    status: IndexStatus,
    f: impl FnOnce(&CoreServices<'_>, SeedIds) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let artifacts = InMemoryArtifactRepository::new();
    let chunks = InMemoryChunkRepository::new();
    let cards = InMemoryCardRepository::new();
    let evidence = InMemoryEvidenceRepository::new();
    let blobs = InMemoryBlobStore::new();
    let events = InMemoryEventLog::new();
    let parser = InMemoryParser::new();
    let search_index = InMemoryFullTextIndex::new();
    let ids = seed_fixture(
        status,
        &artifacts,
        &chunks,
        &cards,
        &evidence,
        &search_index,
    )?;

    let core = CoreServices::new(CorePorts {
        artifacts: &artifacts,
        chunks: &chunks,
        cards: &cards,
        evidence: &evidence,
        events: &events,
        parser: &parser,
        search_index: &search_index,
        blobs: &blobs,
        vector_index: None,
        graph_index: None,
    });
    f(&core, ids)
}

#[test]
fn indexed_artifact_opens_evidence_by_id_and_chunk() -> Result<(), Box<dyn std::error::Error>> {
    with_seed(IndexStatus::Indexed, |core, ids| {
        let (artifact_id, _card_id, chunk_id, _, evidence_id, _) = ids;
        let opened = core.open_evidence(OpenEvidenceInput { evidence_id })?;
        assert_eq!(opened.artifact.id, artifact_id);
        assert_eq!(opened.evidence.id, evidence_id);
        assert_eq!(opened.evidence.excerpt, "alpha-token paragraph.");

        let opened_from_chunk = core.open_chunk_evidence(OpenChunkEvidenceInput { chunk_id })?;
        assert_eq!(opened_from_chunk.evidence.id, evidence_id);
        Ok(())
    })
}

#[test]
fn evidence_opening_rejects_non_indexed_artifacts() -> Result<(), Box<dyn std::error::Error>> {
    for status in [IndexStatus::Pending, IndexStatus::Unindexed] {
        with_seed(status, |core, ids| {
            let error = match core.open_evidence(OpenEvidenceInput { evidence_id: ids.4 }) {
                Ok(_) => return Err("non-indexed evidence unexpectedly opened".into()),
                Err(error) => error,
            };
            assert!(
                matches!(
                    error,
                    CoreError::NotAvailable {
                        kind: "artifact",
                        reason: "not indexed"
                    }
                ),
                "expected NotAvailable error for non-indexed artifact, got: {error}"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn chunk_evidence_uses_canonical_evidence_id() -> Result<(), Box<dyn std::error::Error>> {
    with_seed(IndexStatus::Indexed, |core, ids| {
        let (_, _, _, chunk_id, _, evidence_id) = ids;
        let opened = core.open_chunk_evidence(OpenChunkEvidenceInput { chunk_id })?;
        assert_eq!(opened.evidence.id, evidence_id);
        Ok(())
    })
}
