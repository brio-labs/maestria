use maestria_core::{
    CoreError, CorePorts, CoreServices, OpenChunkEvidenceInput, OpenEvidenceInput,
};
use maestria_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, ContentHash, Evidence, EvidenceId,
    EvidenceKind, IndexStatus, LineRange, LogicalTick, SnapshotRef, SourceSpan, StructureNodeId,
    WebEvidenceMetadata,
};
use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EvidenceRepository,
    FullTextIndex, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
    InMemoryChunkRepository, InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex,
    InMemoryParser, IndexedCard, IndexedChunk,
};

type SeedIds = (ArtifactId, CardId, ChunkId, ChunkId, EvidenceId, EvidenceId);

fn seed_records(
    status: IndexStatus,
    artifacts: &InMemoryArtifactRepository,
    chunks: &InMemoryChunkRepository,
    cards: &InMemoryCardRepository,
    evidence: &InMemoryEvidenceRepository,
    blobs: &InMemoryBlobStore,
    ids: SeedIds,
) -> Result<(), Box<dyn std::error::Error>> {
    let (artifact_id, card_id, chunk_id_0, chunk_id_1, evidence_id_0, evidence_id_1) = ids;
    let source = b"alpha-token paragraph.\nbeta-token paragraph.";
    let snapshot_id = blobs.put(source.to_vec())?;
    let source_hash = maestria_core::content_hash(source);
    artifacts.put(Artifact {
        id: artifact_id,
        title: "multi.md".to_string(),
        chunk_ids: [chunk_id_0, chunk_id_1].into(),
        card_ids: [card_id].into(),
        claim_ids: Default::default(),
        evidence_ids: [evidence_id_0, evidence_id_1].into(),
        index_status: status,
        content_hash: Some(ContentHash::new(source_hash.clone())?),
        parse_status: None,
        security: Default::default(),
    })?;
    cards.put(Card {
        id: card_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::text_span(1, 1)?,
        title: "card-title summary".to_string(),
        body: "card body text".to_string(),
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
            source_span: SourceSpan::text_span(order + 1, order + 1)?,
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
                range: LineRange::new(order, order)?,
                snapshot: SnapshotRef::new(snapshot_id, ContentHash::new(source_hash.clone())?),
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
    blobs: &InMemoryBlobStore,
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
    seed_records(status, artifacts, chunks, cards, evidence, blobs, ids)?;
    seed_indexes(search_index, ids)?;
    Ok(ids)
}
fn with_seed(
    status: IndexStatus,
    f: impl FnOnce(
        &CoreServices<'_>,
        SeedIds,
        &InMemoryArtifactRepository,
        &InMemoryEvidenceRepository,
        &InMemoryBlobStore,
    ) -> Result<(), Box<dyn std::error::Error>>,
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
        &blobs,
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
    f(&core, ids, &artifacts, &evidence, &blobs)
}

#[test]
fn indexed_artifact_opens_evidence_by_id_and_chunk() -> Result<(), Box<dyn std::error::Error>> {
    with_seed(IndexStatus::Indexed, |core, ids, _, _, _| {
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
        with_seed(status, |core, ids, _, _, _| {
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
    with_seed(IndexStatus::Indexed, |core, ids, _, _, _| {
        let (_, _, _, chunk_id, _, evidence_id) = ids;
        let opened = core.open_chunk_evidence(OpenChunkEvidenceInput { chunk_id })?;
        assert_eq!(opened.evidence.id, evidence_id);
        Ok(())
    })
}

#[test]
fn file_snapshot_hash_must_match_owning_artifact() -> Result<(), Box<dyn std::error::Error>> {
    with_seed(IndexStatus::Indexed, |core, ids, artifacts, _, _| {
        let mut artifact = artifacts.get(ids.0)?.ok_or("seed artifact missing")?;
        artifact.content_hash = Some(ContentHash::new(maestria_core::content_hash(
            b"different bytes",
        ))?);
        artifacts.put(artifact)?;

        let error = match core.open_evidence(OpenEvidenceInput { evidence_id: ids.4 }) {
            Ok(_) => return Err("artifact hash mismatch unexpectedly opened".into()),
            Err(error) => error,
        };
        assert!(matches!(error, CoreError::InvalidEvidence { .. }));
        assert!(error.to_string().contains("owning artifact"));
        Ok(())
    })
}

#[test]
fn web_snapshot_hash_must_match_owning_artifact() -> Result<(), Box<dyn std::error::Error>> {
    with_seed(IndexStatus::Indexed, |core, ids, _, evidence, blobs| {
        let bytes = b"h2 web bytes\n";
        let blob_id = blobs.put(bytes.to_vec())?;
        let web_hash = maestria_core::content_hash(bytes);
        let mut record = evidence.get(ids.4)?.ok_or("seed evidence missing")?;
        record.kind = EvidenceKind::WebSnapshot {
            url: "https://example.test/h2".to_string(),
            snapshot: SnapshotRef::new(blob_id, ContentHash::new(web_hash)?),
            fetched_at: LogicalTick::new(2),
            metadata: WebEvidenceMetadata::default(),
        };
        record.excerpt = "h2 web bytes".to_string();
        evidence.replace(record)?;

        let error = match core.open_evidence(OpenEvidenceInput { evidence_id: ids.4 }) {
            Ok(_) => {
                return Err(
                    "web snapshot with a different owning artifact hash unexpectedly opened".into(),
                );
            }
            Err(error) => error,
        };
        assert!(matches!(error, CoreError::InvalidEvidence { .. }));
        assert!(error.to_string().contains("owning artifact"));
        Ok(())
    })
}

#[test]
fn web_snapshot_requires_owning_artifact_hash() -> Result<(), Box<dyn std::error::Error>> {
    with_seed(IndexStatus::Indexed, |core, ids, artifacts, evidence, _| {
        let mut artifact = artifacts.get(ids.0)?.ok_or("seed artifact missing")?;
        artifact.content_hash = None;
        artifacts.put(artifact)?;

        let mut record = evidence.get(ids.4)?.ok_or("seed evidence missing")?;
        let snapshot = match &record.kind {
            EvidenceKind::FileSpan { snapshot, .. } => snapshot.clone(),
            _ => return Err("expected file evidence".into()),
        };
        record.kind = EvidenceKind::WebSnapshot {
            url: "https://example.test/missing-hash".to_string(),
            snapshot,
            fetched_at: LogicalTick::new(2),
            metadata: WebEvidenceMetadata::default(),
        };
        record.excerpt = "alpha-token paragraph.".to_string();
        evidence.replace(record)?;

        let error = match core.open_evidence(OpenEvidenceInput { evidence_id: ids.4 }) {
            Ok(_) => {
                return Err(
                    "web snapshot with missing owning artifact hash unexpectedly opened".into(),
                );
            }
            Err(error) => error,
        };
        assert!(matches!(error, CoreError::InvalidEvidence { .. }));
        assert!(error.to_string().contains("owning artifact"));
        Ok(())
    })
}

#[test]
fn valid_web_snapshot_opens() -> Result<(), Box<dyn std::error::Error>> {
    with_seed(IndexStatus::Indexed, |core, ids, _, evidence, _| {
        let mut record = evidence.get(ids.4)?.ok_or("seed evidence missing")?;
        let snapshot = match &record.kind {
            EvidenceKind::FileSpan { snapshot, .. } => snapshot.clone(),
            _ => return Err("expected file evidence".into()),
        };
        record.kind = EvidenceKind::WebSnapshot {
            url: "https://example.test/valid".to_string(),
            snapshot,
            fetched_at: LogicalTick::new(2),
            metadata: WebEvidenceMetadata::default(),
        };
        record.excerpt = "alpha-token paragraph.".to_string();
        evidence.replace(record)?;

        let opened = core.open_evidence(OpenEvidenceInput { evidence_id: ids.4 })?;
        assert_eq!(opened.evidence.excerpt, "alpha-token paragraph.");
        Ok(())
    })
}

#[test]
fn chunk_opening_rejects_cross_artifact_evidence() -> Result<(), Box<dyn std::error::Error>> {
    with_seed(IndexStatus::Indexed, |core, ids, _, evidence, _| {
        let mut record = evidence.get(ids.4)?.ok_or("seed evidence missing")?;
        record.artifact_id = ArtifactId::new(999);
        evidence.replace(record)?;

        let error = match core.open_chunk_evidence(OpenChunkEvidenceInput { chunk_id: ids.2 }) {
            Ok(_) => return Err("cross-artifact chunk evidence unexpectedly opened".into()),
            Err(error) => error,
        };
        assert!(matches!(error, CoreError::InvalidEvidence { .. }));
        assert!(error.to_string().contains("belongs to artifact"));
        Ok(())
    })
}

#[test]
fn wrong_line_excerpt_is_rejected_after_snapshot_retrieval()
-> Result<(), Box<dyn std::error::Error>> {
    with_seed(IndexStatus::Indexed, |core, ids, _, evidence, _| {
        let mut record = evidence.get(ids.4)?.ok_or("seed evidence missing")?;
        let EvidenceKind::FileSpan { path, snapshot, .. } = &record.kind else {
            return Err("expected file evidence".into());
        };
        record.kind = EvidenceKind::FileSpan {
            path: path.clone(),
            range: LineRange::new(2, 2)?,
            snapshot: snapshot.clone(),
        };
        evidence.replace(record)?;

        let error = match core.open_evidence(OpenEvidenceInput { evidence_id: ids.4 }) {
            Ok(_) => return Err("wrong-line excerpt unexpectedly opened".into()),
            Err(error) => error,
        };
        assert!(error.to_string().contains("selected lines"));
        Ok(())
    })
}

#[test]
fn malformed_utf8_snapshot_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    with_seed(
        IndexStatus::Indexed,
        |core, ids, artifacts, evidence, blobs| {
            let bytes = vec![0xff, 0xfe];
            let blob_id = blobs.put(bytes.clone())?;
            let hash = maestria_core::content_hash(&bytes);
            let mut artifact = artifacts.get(ids.0)?.ok_or("seed artifact missing")?;
            artifact.content_hash = Some(ContentHash::new(hash.clone())?);
            artifacts.put(artifact)?;

            let mut record = evidence.get(ids.4)?.ok_or("seed evidence missing")?;
            record.kind = EvidenceKind::FileSpan {
                path: "multi.md".to_string(),
                range: LineRange::new(1, 1)?,
                snapshot: SnapshotRef::new(blob_id, ContentHash::new(hash)?),
            };
            record.excerpt.clear();
            evidence.replace(record)?;

            let error = match core.open_evidence(OpenEvidenceInput { evidence_id: ids.4 }) {
                Ok(_) => return Err("malformed UTF-8 unexpectedly opened".into()),
                Err(error) => error,
            };
            assert!(error.to_string().contains("valid UTF-8"));
            Ok(())
        },
    )
}

#[test]
fn retrieved_blob_hash_mismatch_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    with_seed(
        IndexStatus::Indexed,
        |core, ids, artifacts, evidence, blobs| {
            let bytes = b"tampered bytes";
            let blob_id = blobs.put(bytes.to_vec())?;
            let expected_hash = maestria_core::content_hash(b"expected bytes");
            let mut artifact = artifacts.get(ids.0)?.ok_or("seed artifact missing")?;
            artifact.content_hash = Some(ContentHash::new(expected_hash.clone())?);
            artifacts.put(artifact)?;

            let mut record = evidence.get(ids.4)?.ok_or("seed evidence missing")?;
            record.kind = EvidenceKind::FileSpan {
                path: "multi.md".to_string(),
                range: LineRange::new(1, 1)?,
                snapshot: SnapshotRef::new(blob_id, ContentHash::new(expected_hash)?),
            };
            record.excerpt.clear();
            evidence.replace(record)?;

            let error = match core.open_evidence(OpenEvidenceInput { evidence_id: ids.4 }) {
                Ok(_) => return Err("blob hash mismatch unexpectedly opened".into()),
                Err(error) => error,
            };
            assert!(error.to_string().contains("hash mismatch"));
            Ok(())
        },
    )
}
