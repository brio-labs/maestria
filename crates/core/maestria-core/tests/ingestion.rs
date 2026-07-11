use std::path::PathBuf;

use maestria_core::{CorePorts, CoreServices, IngestFileInput};
use maestria_domain::{ArtifactId, EvidenceKind, LogicalTick};
use maestria_ports::{
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository, InMemoryChunkRepository,
    InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryParser,
};

#[test]
fn ingest_reuses_existing_artifact_data_when_unchanged() -> Result<(), Box<dyn std::error::Error>> {
    let artifacts = InMemoryArtifactRepository::new();
    let chunks = InMemoryChunkRepository::new();
    let cards = InMemoryCardRepository::new();
    let evidence = InMemoryEvidenceRepository::new();
    let events = InMemoryEventLog::new();
    let parser = InMemoryParser::new();
    let search_index = InMemoryFullTextIndex::new();
    let blobs = InMemoryBlobStore::new();
    let core = CoreServices::new(CorePorts {
        artifacts: &artifacts,
        chunks: &chunks,
        cards: &cards,
        evidence: &evidence,
        events: &events,
        parser: &parser,
        search_index: &search_index,
        blobs: &blobs,
    });

    let path = PathBuf::from("notes/project.md");
    let ingested = core.ingest_file_from_bytes(IngestFileInput {
        path: path.clone(),
        bytes: b"# Project\n\nLocal brain ingestion should find retrieval evidence.".to_vec(),
        observed_at: LogicalTick::new(7),
        artifact_id: Some(ArtifactId::new(42)),
    })?;

    assert_eq!(ingested.artifact.id, ArtifactId::new(42));
    assert_eq!(ingested.chunks.len(), 1);
    assert_eq!(ingested.evidence.len(), 1);
    assert_eq!(ingested.chunks[0].artifact_id, ingested.artifact.id);
    assert_eq!(ingested.evidence[0].artifact_id, ingested.artifact.id);

    let events_before =
        maestria_ports::EventLog::scan(&events, maestria_ports::EventFilter { artifact_id: None })?
            .len();
    let repeated = core.ingest_file_from_bytes(IngestFileInput {
        path: path.clone(),
        bytes: b"# Project\n\nLocal brain ingestion should find retrieval evidence.".to_vec(),
        observed_at: LogicalTick::new(8),
        artifact_id: Some(ArtifactId::new(42)),
    })?;

    let changed = core.ingest_file_from_bytes(IngestFileInput {
        path,
        bytes: b"# Project\n\nA changed source body.".to_vec(),
        observed_at: LogicalTick::new(9),
        artifact_id: Some(ArtifactId::new(42)),
    });
    assert!(matches!(
        changed,
        Err(maestria_core::CoreError::InvalidInput { ref message })
            if message.contains("different or untraceable")
    ));

    assert!(repeated.unchanged);
    assert_eq!(repeated.artifact, ingested.artifact);
    assert_eq!(repeated.chunks, ingested.chunks);
    assert_eq!(repeated.evidence, ingested.evidence);
    assert_eq!(
        maestria_ports::EventLog::scan(&events, maestria_ports::EventFilter { artifact_id: None })?
            .len(),
        events_before
    );

    Ok(())
}

#[test]
fn ingest_with_in_memory_ports_is_queryable_and_openable() -> Result<(), Box<dyn std::error::Error>>
{
    let artifacts = InMemoryArtifactRepository::new();
    let chunks = InMemoryChunkRepository::new();
    let cards = InMemoryCardRepository::new();
    let evidence = InMemoryEvidenceRepository::new();
    let events = InMemoryEventLog::new();
    let parser = InMemoryParser::new();
    let search_index = InMemoryFullTextIndex::new();
    let blobs = InMemoryBlobStore::new();
    let core = CoreServices::new(CorePorts {
        artifacts: &artifacts,
        chunks: &chunks,
        cards: &cards,
        evidence: &evidence,
        events: &events,
        parser: &parser,
        search_index: &search_index,
        blobs: &blobs,
    });

    let ingested = core.ingest_file_from_bytes(IngestFileInput {
        path: PathBuf::from("notes/project.md"),
        bytes: b"# Project\n\nLocal brain ingestion should find retrieval evidence.".to_vec(),
        observed_at: LogicalTick::new(7),
        artifact_id: Some(ArtifactId::new(7)),
    })?;

    let search = core.search(maestria_core::SearchInput {
        query: "retrieval".to_string(),
        limit: 5,
    })?;
    assert_eq!(search.hits.len(), 1);
    assert_eq!(search.hits[0].artifact.id, ingested.artifact.id);
    assert_eq!(search.hits[0].chunk.id, ingested.chunks[0].id);
    let hit_evidence = &search.hits[0].evidence;
    assert_eq!(hit_evidence.id, ingested.evidence[0].id);

    let opened = core.open_evidence(maestria_core::OpenEvidenceInput {
        evidence_id: hit_evidence.id,
    })?;
    assert_eq!(opened.artifact.id, ingested.artifact.id);
    assert_eq!(opened.evidence.id, hit_evidence.id);
    match opened.evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            content_hash,
        } => {
            assert_eq!(path, "notes/project.md");
            assert_eq!(range.start, 1);
            assert!(range.end >= range.start);
            assert_eq!(content_hash, ingested.content_hash);
        }
        other => panic!("expected file evidence, got {other:?}"),
    }

    Ok(())
}
