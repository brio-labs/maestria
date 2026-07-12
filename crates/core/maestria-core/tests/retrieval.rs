use maestria_core::{
    CorePorts, CoreServices, OpenChunkEvidenceInput, OpenEvidenceInput, SearchInput,
};
use maestria_domain::{Artifact, ArtifactId, Chunk, ChunkId, Evidence, EvidenceKind, IndexStatus};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, FullTextIndex,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository, InMemoryChunkRepository,
    InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryParser,
    IndexedChunk,
};

/// Seed an artifact, chunks, evidence, and full-text entries directly through
/// in-memory adapters, then wrap them in a `CoreServices` to exercise retrieval.
fn seed_and_build_services<'a>(ports: CorePorts<'a>) -> CoreServices<'a> {
    CoreServices::new(ports)
}

#[test]
fn search_and_open_evidence_with_directly_seeded_artifact() -> Result<(), Box<dyn std::error::Error>>
{
    // 1. Create adapters and seed directly — no ingestion path.
    let artifact_repo = InMemoryArtifactRepository::new();
    let chunk_repo = InMemoryChunkRepository::new();
    let evidence_repo = InMemoryEvidenceRepository::new();
    let blob_store = InMemoryBlobStore::new();
    let search_index = InMemoryFullTextIndex::new();
    let events = InMemoryEventLog::new();
    let parser = InMemoryParser::new();
    let cards = InMemoryCardRepository::new();
    let artifact_id = ArtifactId::new(7);
    let chunk_id_0 = ChunkId::new(701);
    let chunk_id_1 = ChunkId::new(702);
    let evidence_id_0 = maestria_domain::evidence_id_for(artifact_id, 0);
    let evidence_id_1 = maestria_domain::evidence_id_for(artifact_id, 1);

    let source_text = "alpha-token paragraph.\n\nbeta-token paragraph.\n";
    let blob_id = blob_store.put(source_text.as_bytes().to_vec())?;

    // Seed an artifact.
    artifact_repo.put(Artifact {
        id: artifact_id,
        title: "multi.md".to_string(),
        chunk_ids: [chunk_id_0, chunk_id_1].into(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: [evidence_id_0, evidence_id_1].into(),
        index_status: IndexStatus::Indexed,
        content_hash: None,
    })?;

    // Seed chunks.
    let chunk_0 = Chunk {
        id: chunk_id_0,
        artifact_id,
        order: 0,
        text: "alpha-token paragraph.".to_string(),
    };
    let chunk_1 = Chunk {
        id: chunk_id_1,
        artifact_id,
        order: 1,
        text: "beta-token paragraph.".to_string(),
    };
    chunk_repo.put(chunk_0.clone())?;
    chunk_repo.put(chunk_1.clone())?;

    // Seed evidence.
    let content_hash = maestria_core::content_hash(source_text.as_bytes());
    let evidence_0 = Evidence {
        id: evidence_id_0,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "notes/multi.md".to_string(),
            range: maestria_domain::ContentRange { start: 1, end: 1 },
            content_hash: content_hash.clone(),
            snapshot: Some(blob_id),
        },
        excerpt: "alpha-token paragraph.".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
    };
    let evidence_1 = Evidence {
        id: evidence_id_1,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "notes/multi.md".to_string(),
            range: maestria_domain::ContentRange { start: 3, end: 3 },
            content_hash: content_hash.clone(),
            snapshot: Some(blob_id),
        },
        excerpt: "beta-token paragraph.".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
    };
    evidence_repo.put(evidence_0.clone())?;
    evidence_repo.put(evidence_1.clone())?;

    // Index chunks for full-text search.
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

    // 2. Build CoreServices around the seeded adapters.
    let core = seed_and_build_services(CorePorts {
        artifacts: &artifact_repo,
        chunks: &chunk_repo,
        cards: &cards,
        evidence: &evidence_repo,
        events: &events,
        parser: &parser,
        search_index: &search_index,
        blobs: &blob_store,
    });

    // 3. Exercise retrieval APIs.

    // Search: "beta-token" should match chunk 1.
    let search_result = core.search(SearchInput {
        query: "beta-token".to_string(),
        limit: 5,
    })?;
    assert_eq!(search_result.hits.len(), 1);
    let hit = &search_result.hits[0];
    assert_eq!(hit.artifact.id, artifact_id);
    assert_eq!(hit.chunk.id, chunk_id_1);
    assert_eq!(hit.evidence.id, evidence_id_1);
    assert_eq!(hit.evidence.excerpt, "beta-token paragraph.");

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
