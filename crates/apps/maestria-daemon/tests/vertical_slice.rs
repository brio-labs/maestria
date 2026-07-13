use maestria_blob_fs::FsBlobStore;
use maestria_core::{
    CorePorts, CoreServices, InstanceLayout, InstanceService, OpenEvidenceInput, SearchInput,
};
use maestria_domain::{
    ArtifactDetected, ArtifactId, ChunkId, DomainInput, EvidenceId, IndexStatus, KernelState,
    content_hash,
};
use maestria_governance::AutonomyProfile;
use maestria_parsers::ParserRegistry;
use maestria_ports::{ArtifactRepository, EventFilter, EventLog};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
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

fn setup_layout(root: &Path, notes_dir: &Path) -> InstanceLayout {
    let plan = InstanceService::init_instance_with_roots(
        root.to_path_buf(),
        vec![notes_dir.to_path_buf()],
    )
    .expect("init instance");
    for dir in &plan.directories {
        fs::create_dir_all(dir).expect("create dir");
    }
    fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes()).expect("write manifest");
    plan.layout
}

async fn wait_for_indexed(db_path: &Path, artifact_id: ArtifactId) -> bool {
    for _ in 0..60 {
        if let Ok(db) = SqliteStore::open(db_path)
            && let Ok(Some(artifact)) = db.get(artifact_id)
            && artifact.index_status == IndexStatus::Indexed
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

struct TestEnv {
    layout: InstanceLayout,
    input_tx: tokio::sync::mpsc::Sender<DomainInput>,
    shutdown_token: CancellationToken,
    runtime_handle: tokio::task::JoinHandle<()>,
    artifact_id: ArtifactId,
    hash: String,
    bytes: Vec<u8>,
    source_path: String,
}

fn prepare_test_env(tmp: &TempDir) -> TestEnv {
    let root = tmp.path();
    let notes = root.join("notes");
    fs::create_dir_all(&notes).expect("create notes");

    // Write test content
    let test_path = notes.join("graph-rag.md");
    let source_path = test_path.to_string_lossy().to_string();
    let test_content = "# GraphRAG Survey\n\nGraph Retrieval-Augmented Generation combines knowledge graphs with RAG to improve multi-hop reasoning.\n";
    fs::write(&test_path, test_content).expect("write test file");

    let layout = setup_layout(root, &notes);
    let bytes = fs::read(&test_path).expect("read test file");
    let hash = content_hash(&bytes);

    // Build runtime with StrictResearch profile to allow indexing
    let (runtime, input_tx, input_rx, shutdown_token) = maestria_daemon::build_runtime(
        &layout,
        KernelState::new(),
        AutonomyProfile::StrictResearch,
    )
    .expect("build runtime");

    let shutdown = shutdown_token.clone();
    let runtime_handle = tokio::spawn(async move {
        runtime.run(input_rx, shutdown).await;
    });

    let artifact_id = ArtifactId::new(1);

    TestEnv {
        layout,
        input_tx,
        shutdown_token,
        runtime_handle,
        artifact_id,
        hash,
        bytes,
        source_path,
    }
}

struct IndexResult {
    evidence_id: EvidenceId,
    first_chunk_id: ChunkId,
    event_count: usize,
}

async fn index_and_verify_artifact(
    input_tx: &tokio::sync::mpsc::Sender<DomainInput>,
    db_path: &Path,
    artifact_id: ArtifactId,
    hash: &str,
    bytes: &[u8],
    source_path: &str,
) -> IndexResult {
    // === INDEX via DomainInput ===
    input_tx
        .send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "graph-rag.md".to_string(),
            source_path: source_path.to_string(),
            source_bytes: bytes.to_vec(),
            content_hash: hash.to_string(),
        }))
        .await
        .expect("send ArtifactDetected");

    // Wait for IndexStatus::Indexed by polling the event log through a second connection
    assert!(
        wait_for_indexed(db_path, artifact_id).await,
        "artifact should reach Indexed status"
    );

    // Verify artifact state after indexing
    let check_db = SqliteStore::open(db_path).expect("open check db");
    let artifact = check_db
        .get(artifact_id)
        .expect("get artifact")
        .expect("artifact should exist");
    assert_eq!(artifact.index_status, IndexStatus::Indexed);
    assert_eq!(artifact.content_hash.as_deref(), Some(hash));
    assert!(!artifact.chunk_ids.is_empty(), "should have chunks");
    assert!(!artifact.evidence_ids.is_empty(), "should have evidence");

    let evidence_id = *artifact.evidence_ids.first().expect("should have evidence");
    let first_chunk_id = *artifact.chunk_ids.first().expect("should have chunk");

    // Count events for idempotence check
    let event_count = EventLog::scan(
        &check_db,
        EventFilter {
            artifact_id: Some(artifact_id),
        },
    )
    .expect("scan events")
    .len();
    drop(check_db);

    IndexResult {
        evidence_id,
        first_chunk_id,
        event_count,
    }
}

async fn attempt_idempotent_reindex(
    input_tx: &tokio::sync::mpsc::Sender<DomainInput>,
    db_path: &Path,
    artifact_id: ArtifactId,
    hash: &str,
    bytes: &[u8],
    source_path: &str,
    expected_event_count: usize,
) {
    // === IDEMPOTENT RE-INDEX ===
    input_tx
        .send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "graph-rag.md".to_string(),
            source_path: source_path.to_string(),
            source_bytes: bytes.to_vec(),
            content_hash: hash.to_string(),
        }))
        .await
        .expect("send duplicate ArtifactDetected");

    // Brief wait for the runtime to process the no-op input
    tokio::time::sleep(Duration::from_millis(300)).await;

    let check_db = SqliteStore::open(db_path).expect("re-open check db");
    let event_count_after = EventLog::scan(
        &check_db,
        EventFilter {
            artifact_id: Some(artifact_id),
        },
    )
    .expect("scan events after")
    .len();
    assert_eq!(
        event_count_after, expected_event_count,
        "re-index should not produce new events for unchanged content"
    );
    drop(check_db);
}

fn search_and_open_evidence_after_restart(
    layout: &InstanceLayout,
    artifact_id: ArtifactId,
    evidence_id: EvidenceId,
    first_chunk_id: ChunkId,
) {
    // === RESTART SIMULATION ===
    let sqlite = SqliteStore::open(&layout.database_path).expect("re-open sqlite");
    let blobs = FsBlobStore::open(&layout.blobs_dir).expect("re-open blobs");
    let search = TantivyFullTextIndex::open(&layout.full_text_index_dir).expect("re-open search");
    let parser = ParserRegistry::with_defaults();

    let core = CoreServices::new(CorePorts {
        artifacts: &sqlite,
        chunks: &sqlite,
        cards: &sqlite,
        evidence: &sqlite,
        events: &sqlite,
        parser: &parser,
        search_index: &search,
        blobs: &blobs,
        embedding_provider: None,
        vector_index: None,
    });

    // === SEARCH AFTER RESTART ===
    let results = core
        .search(SearchInput {
            query: "GraphRAG knowledge graphs".into(),
            limit: 10,
        })
        .expect("search after restart");

    assert!(
        !results.pack.chunks.is_empty(),
        "search after restart should return results"
    );
    assert!(
        results.pack.chunks[0].chunk.text.contains("GraphRAG")
            || results.pack.chunks[0].chunk.text.contains("graph"),
        "result should contain query terms"
    );
    assert_eq!(
        results.pack.chunks[0].chunk.id, first_chunk_id,
        "chunk id should be stable across restart"
    );

    // === OPEN EVIDENCE AFTER RESTART ===
    let ev = core
        .open_evidence(OpenEvidenceInput { evidence_id })
        .expect("open evidence after restart");

    assert_eq!(
        ev.evidence.id, evidence_id,
        "evidence should resolve after restart"
    );
    assert!(
        ev.evidence.excerpt.contains("GraphRAG"),
        "excerpt should contain content"
    );
    assert_eq!(ev.artifact.id, artifact_id, "artifact should match");
}

#[tokio::test]
async fn vertical_slice_init_index_search_evidence() {
    let tmp = TempDir::new();
    let env = prepare_test_env(&tmp);

    let IndexResult {
        evidence_id,
        first_chunk_id,
        event_count,
    } = index_and_verify_artifact(
        &env.input_tx,
        &env.layout.database_path,
        env.artifact_id,
        &env.hash,
        &env.bytes,
        &env.source_path,
    )
    .await;

    attempt_idempotent_reindex(
        &env.input_tx,
        &env.layout.database_path,
        env.artifact_id,
        &env.hash,
        &env.bytes,
        &env.source_path,
        event_count,
    )
    .await;

    // === STOP RUNTIME ===
    env.shutdown_token.cancel();
    env.runtime_handle.await.expect("runtime task completes");

    search_and_open_evidence_after_restart(
        &env.layout,
        env.artifact_id,
        evidence_id,
        first_chunk_id,
    );
}
