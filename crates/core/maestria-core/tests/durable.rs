use std::path::PathBuf;

use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, IngestFileInput, OpenEvidenceInput, SearchInput};
use maestria_domain::{EvidenceKind, LogicalTick};
use maestria_parsers::ParserRegistry;
use maestria_ports::{EventFilter, EventLog};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use tempfile::tempdir;

fn services<'a>(
    sqlite: &'a SqliteStore,
    blobs: &'a FsBlobStore,
    search: &'a TantivyFullTextIndex,
    parser: &'a ParserRegistry,
) -> CoreServices<'a> {
    CoreServices::new(CorePorts {
        artifacts: sqlite,
        chunks: sqlite,
        cards: sqlite,
        evidence: sqlite,
        events: sqlite,
        parser,
        search_index: search,
        blobs,
    })
}

#[test]
fn real_adapters_survive_restart_and_unchanged_reindex() -> Result<(), Box<dyn std::error::Error>> {
    let instance = tempdir()?;
    let database_path = instance.path().join("maestria.db");
    let blobs_path = instance.path().join("blobs");
    let index_path = instance.path().join("index");
    let source_path = PathBuf::from("notes/restart.md");
    let bytes = b"# Durable indexing\n\nEvidence survives a process restart.".to_vec();

    let first = {
        let sqlite = SqliteStore::open(&database_path)?;
        let blobs = FsBlobStore::open(&blobs_path)?;
        let search = TantivyFullTextIndex::open(&index_path)?;
        let parser = ParserRegistry::with_defaults();
        let core = services(&sqlite, &blobs, &search, &parser);
        let output = core.ingest_file_from_bytes(IngestFileInput {
            path: source_path.clone(),
            bytes: bytes.clone(),
            observed_at: LogicalTick::new(1),
            artifact_id: None,
        })?;
        let events = sqlite.scan(EventFilter { artifact_id: None })?;
        assert!(!output.unchanged);
        assert_eq!(events.len(), 5);
        output
    };

    let sqlite = SqliteStore::open(&database_path)?;
    let blobs = FsBlobStore::open(&blobs_path)?;
    let search = TantivyFullTextIndex::open(&index_path)?;
    let parser = ParserRegistry::with_defaults();
    let core = services(&sqlite, &blobs, &search, &parser);

    let results = core.search(SearchInput {
        query: "restart".to_string(),
        limit: 5,
    })?;
    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].artifact.id, first.artifact.id);
    assert_eq!(results.hits[0].evidence.id, first.evidence[0].id);

    let opened = core.open_evidence(OpenEvidenceInput {
        evidence_id: first.evidence[0].id,
    })?;
    assert_eq!(opened.evidence.id, first.evidence[0].id);
    match opened.evidence.kind {
        EvidenceKind::FileSpan {
            path, content_hash, ..
        } => {
            assert_eq!(path, source_path.display().to_string());
            assert_eq!(content_hash, first.content_hash);
        }
        other => panic!("expected file evidence, got {other:?}"),
    }

    let repeated = core.ingest_file_from_bytes(IngestFileInput {
        path: source_path,
        bytes,
        observed_at: LogicalTick::new(2),
        artifact_id: None,
    })?;
    assert!(repeated.unchanged);
    assert_eq!(repeated.artifact, first.artifact);
    assert_eq!(sqlite.scan(EventFilter { artifact_id: None })?.len(), 5);

    Ok(())
}
