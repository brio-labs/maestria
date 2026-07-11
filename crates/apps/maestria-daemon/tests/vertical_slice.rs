use maestria_blob_fs::FsBlobStore;
use maestria_core::{
    CorePorts, CoreServices, IngestFileInput, InstanceLayout, InstanceService, OpenEvidenceInput,
    SearchInput,
};
use maestria_domain::LogicalTick;
use maestria_parsers::ParserRegistry;
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = env::temp_dir().join(format!("maestria-test-{}", std::process::id()));
        // Best-effort cleanup of any previous run that left the directory
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn setup_instance(
    root: &Path,
    notes_dir: &Path,
) -> (
    InstanceLayout,
    SqliteStore,
    FsBlobStore,
    TantivyFullTextIndex,
    ParserRegistry,
) {
    let plan = InstanceService::init_instance_with_roots(
        root.to_path_buf(),
        vec![notes_dir.to_path_buf()],
    )
    .expect("init instance");
    for dir in &plan.directories {
        fs::create_dir_all(dir).expect("create dir");
    }
    fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes()).expect("write manifest");

    let sqlite = SqliteStore::open(&plan.layout.database_path).expect("open sqlite");
    let blobs = FsBlobStore::open(&plan.layout.blobs_dir).expect("open blobs");
    let search = TantivyFullTextIndex::open(&plan.layout.full_text_index_dir).expect("open search");
    let parser = ParserRegistry::with_defaults();

    (plan.layout, sqlite, blobs, search, parser)
}

#[test]
fn vertical_slice_init_index_search_evidence() {
    let tmp = TempDir::new();
    let root = tmp.path();
    let notes = root.join("notes");
    fs::create_dir_all(&notes).expect("create notes");

    // Write test content
    let test_path = notes.join("graph-rag.md");
    let test_content = "# GraphRAG Survey\n\nGraph Retrieval-Augmented Generation combines knowledge graphs with RAG to improve multi-hop reasoning.\n";
    fs::write(&test_path, test_content).expect("write test file");

    let (layout, sqlite, blobs, search, parser) = setup_instance(root, &notes);

    let core = CoreServices::new(CorePorts {
        artifacts: &sqlite,
        chunks: &sqlite,
        cards: &sqlite,
        evidence: &sqlite,
        events: &sqlite,
        parser: &parser,
        search_index: &search,
        blobs: &blobs,
    });

    // === INDEX ===
    let bytes = fs::read(&test_path).expect("read test file");
    let output = core
        .ingest_file_from_bytes(IngestFileInput {
            path: test_path.clone(),
            bytes,
            observed_at: LogicalTick::new(1),
            artifact_id: None,
        })
        .expect("ingest");

    assert!(!output.unchanged, "first index should detect new content");
    assert_eq!(output.chunks.len(), 1, "should have one chunk");
    assert_eq!(output.evidence.len(), 1, "should have one evidence");
    assert!(!output.content_hash.is_empty(), "should have content hash");

    let evidence_id = output.evidence[0].id;
    let first_chunk_id = output.chunks[0].id;

    // === SEARCH ===
    let results = core
        .search(SearchInput {
            query: "GraphRAG knowledge graphs".into(),
            limit: 10,
        })
        .expect("search");

    assert!(!results.hits.is_empty(), "search should return results");
    assert!(
        results.hits[0].chunk.text.contains("GraphRAG")
            || results.hits[0].chunk.text.contains("graph"),
        "result should contain query terms"
    );

    // === OPEN EVIDENCE ===
    let ev = core
        .open_evidence(OpenEvidenceInput { evidence_id })
        .expect("open evidence");

    assert_eq!(ev.evidence.id, evidence_id, "evidence id should match");
    assert!(
        ev.evidence.excerpt.contains("GraphRAG"),
        "excerpt should contain content"
    );
    assert_eq!(ev.artifact.id, output.artifact.id, "artifact should match");

    // === IDEMPOTENT RE-INDEX ===
    let bytes2 = fs::read(&test_path).expect("re-read test file");
    let output2 = core
        .ingest_file_from_bytes(IngestFileInput {
            path: test_path.clone(),
            bytes: bytes2,
            observed_at: LogicalTick::new(2),
            artifact_id: None,
        })
        .expect("re-ingest");

    assert!(
        output2.unchanged,
        "re-index should detect unchanged content"
    );
    assert_eq!(
        output2.content_hash, output.content_hash,
        "content hash should be stable"
    );
    assert_eq!(
        output2.chunks.len(),
        output.chunks.len(),
        "same chunk count"
    );

    // === RESTART SIMULATION ===
    // Drop everything and re-open to simulate daemon restart
    // core is consumed by drop (CoreServices does not implement Drop)
    drop(sqlite);
    drop(blobs);
    drop(search);

    let sqlite2 = SqliteStore::open(&layout.database_path).expect("re-open sqlite");
    let blobs2 = FsBlobStore::open(&layout.blobs_dir).expect("re-open blobs");
    let search2 = TantivyFullTextIndex::open(&layout.full_text_index_dir).expect("re-open search");
    let parser2 = ParserRegistry::with_defaults();

    let core2 = CoreServices::new(CorePorts {
        artifacts: &sqlite2,
        chunks: &sqlite2,
        cards: &sqlite2,
        evidence: &sqlite2,
        events: &sqlite2,
        parser: &parser2,
        search_index: &search2,
        blobs: &blobs2,
    });

    // === SEARCH AFTER RESTART ===
    let results2 = core2
        .search(SearchInput {
            query: "GraphRAG knowledge graphs".into(),
            limit: 10,
        })
        .expect("search after restart");

    assert!(
        !results2.hits.is_empty(),
        "search after restart should return results"
    );
    assert_eq!(
        results2.hits[0].chunk.id, first_chunk_id,
        "chunk id should be stable across restart"
    );

    // === OPEN EVIDENCE AFTER RESTART ===
    let ev2 = core2
        .open_evidence(OpenEvidenceInput { evidence_id })
        .expect("open evidence after restart");

    assert_eq!(
        ev2.evidence.id, evidence_id,
        "evidence should resolve after restart"
    );
    assert_eq!(
        ev2.evidence.excerpt, ev.evidence.excerpt,
        "excerpt should be identical"
    );
}
