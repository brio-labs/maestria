use anyhow::Error as AnyhowError;
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, InstanceLayout, InstanceService, OpenEvidenceInput};
use maestria_domain::{
    ArtifactDetected, ArtifactId, CardId, ChunkId, ContentHash, DomainInput, EvidenceId,
    IndexStatus, Relation, RelationEndpoint, RelationId, RelationKind, content_hash,
};
use maestria_governance::{AutonomyProfile, RetrievalSecurityPolicy};
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_parsers::ParserRegistry;
use maestria_ports::{
    ArtifactRepository, EffectJournal, EmbeddingProvenance, EventFilter, EventLog, GraphIndex,
    GraphRelationQuery, VectorEmbedding, VectorIndex, VectorSearchQuery,
};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
struct TempDir(PathBuf);

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

fn search_budget(
    limit: u64,
) -> Result<maestria_domain::SearchExecutionBudget, maestria_domain::SearchCompatibilityError> {
    maestria_domain::SearchExecutionBudget::new(limit, 10_000, 100_000, 0)
}

impl TempDir {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let suffix = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = env::temp_dir().join(format!("maestria-test-{}-{suffix}", std::process::id()));
        // Best-effort cleanup of any previous run that left the directory
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
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

fn setup_layout(
    root: &Path,
    notes_dir: &Path,
) -> Result<InstanceLayout, Box<dyn std::error::Error>> {
    let plan = InstanceService::init_instance_with_roots(
        root.to_path_buf(),
        vec![notes_dir.to_path_buf()],
    )?;
    for dir in &plan.directories {
        fs::create_dir_all(dir)?;
    }
    fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;
    Ok(plan.layout)
}

async fn wait_for_indexed(
    db_path: &Path,
    artifact_id: ArtifactId,
) -> Result<bool, Box<dyn std::error::Error>> {
    for _ in 0..60 {
        if let Ok(db) = SqliteStore::open(db_path)
            && let Ok(Some(artifact)) = db.get(artifact_id)
            && artifact.index_status == IndexStatus::Indexed
        {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(false)
}

struct TestEnv {
    layout: InstanceLayout,
    session: Option<maestria_daemon::MutationSession>,
    artifact_id: ArtifactId,
    hash: String,
    bytes: Vec<u8>,
    source_path: String,
}

async fn prepare_test_env(tmp: &TempDir) -> Result<TestEnv, Box<dyn std::error::Error>> {
    let root = tmp.path();
    let notes = root.join("notes");
    fs::create_dir_all(&notes)?;

    // Write test content
    let test_path = notes.join("graph-rag.md");
    let source_path = test_path.to_string_lossy().to_string();
    let test_content = "# GraphRAG Survey\n\nGraph Retrieval-Augmented Generation combines knowledge graphs with RAG to improve multi-hop reasoning.\n";
    fs::write(&test_path, test_content)?;

    let layout = setup_layout(root, &notes)?;
    let bytes = fs::read(&test_path)?;
    let hash = content_hash(&bytes);

    // Start a correlated mutation session with startup recovery applied.
    let session =
        maestria_daemon::MutationSession::start(layout.clone(), AutonomyProfile::StrictResearch)
            .await?;

    let artifact_id = ArtifactId::new(1);
    Ok(TestEnv {
        layout,
        session: Some(session),
        artifact_id,
        hash,
        bytes,
        source_path,
    })
}

async fn finish_session<T>(
    session: Option<maestria_daemon::MutationSession>,
    operation: Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    let session =
        session.ok_or_else(|| std::io::Error::other("mutation session already finished"))?;
    session
        .finish(operation.map_err(|error| AnyhowError::msg(error.to_string())))
        .await
        .map_err(|error| std::io::Error::other(error.to_string()).into())
}

struct IndexResult {
    evidence_id: EvidenceId,
    first_chunk_id: ChunkId,
    event_count: usize,
}

async fn index_and_verify_artifact(
    session: &maestria_daemon::MutationSession,
    db_path: &Path,
    artifact_id: ArtifactId,
    hash: &str,
    bytes: &[u8],
    source_path: &str,
) -> Result<IndexResult, Box<dyn std::error::Error>> {
    // === INDEX via correlated DomainInput submission ===
    session
        .submit(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "graph-rag.md".to_string(),
            source_path: source_path.to_string(),
            source_bytes: bytes.to_vec(),
            content_hash: ContentHash::new(hash.to_string())?,
        }))
        .await?;

    // Wait for IndexStatus::Indexed by polling the event log through a second connection
    assert!(
        wait_for_indexed(db_path, artifact_id).await?,
        "artifact should reach Indexed status"
    );

    // Verify artifact state after indexing
    let check_db = SqliteStore::open(db_path)?;
    let artifact = check_db
        .get(artifact_id)?
        .ok_or_else(|| std::io::Error::other("indexed artifact missing from store"))?;
    assert_eq!(artifact.index_status, IndexStatus::Indexed);
    assert_eq!(
        artifact.content_hash.as_ref().map(|h| h.as_str()),
        Some(hash)
    );
    assert!(!artifact.chunk_ids.is_empty(), "should have chunks");
    assert!(!artifact.evidence_ids.is_empty(), "should have evidence");

    let evidence_id = *artifact
        .evidence_ids
        .first()
        .ok_or_else(|| std::io::Error::other("indexed artifact has no evidence"))?;
    let first_chunk_id = *artifact
        .chunk_ids
        .first()
        .ok_or_else(|| std::io::Error::other("indexed artifact has no chunks"))?;

    // Count events for idempotence check
    let event_count = EventLog::scan(
        &check_db,
        EventFilter {
            artifact_id: Some(artifact_id),
        },
    )?
    .len();
    drop(check_db);

    Ok(IndexResult {
        evidence_id,
        first_chunk_id,
        event_count,
    })
}

async fn attempt_idempotent_reindex(
    session: &maestria_daemon::MutationSession,
    db_path: &Path,
    artifact_id: ArtifactId,
    hash: &str,
    bytes: &[u8],
    source_path: &str,
    expected_event_count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    // === IDEMPOTENT RE-INDEX ===
    session
        .submit(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "graph-rag.md".to_string(),
            source_path: source_path.to_string(),
            source_bytes: bytes.to_vec(),
            content_hash: ContentHash::new(hash.to_string())?,
        }))
        .await?;

    // Brief wait for the runtime to process the no-op input
    tokio::time::sleep(Duration::from_millis(300)).await;

    let check_db = SqliteStore::open(db_path)?;
    let event_count_after = EventLog::scan(
        &check_db,
        EventFilter {
            artifact_id: Some(artifact_id),
        },
    )?
    .len();
    assert_eq!(
        event_count_after, expected_event_count,
        "re-index should not produce new events for unchanged content"
    );
    drop(check_db);
    Ok(())
}

async fn search_and_open_evidence_after_restart(
    layout: &InstanceLayout,
    artifact_id: ArtifactId,
    evidence_id: EvidenceId,
    _first_chunk_id: ChunkId,
) -> Result<(), Box<dyn std::error::Error>> {
    let sqlite = SqliteStore::open(&layout.database_path)?;
    let blobs = FsBlobStore::open(&layout.blobs_dir)?;
    let search = TantivyFullTextIndex::open(&layout.full_text_index_dir)?;
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
        vector_index: None,
        graph_index: None,
    });

    let ev = core.open_evidence(OpenEvidenceInput { evidence_id })?;
    assert_eq!(
        ev.evidence.id, evidence_id,
        "evidence should resolve after restart"
    );
    assert!(
        ev.evidence.excerpt.contains("GraphRAG"),
        "excerpt should contain content"
    );
    assert_eq!(ev.artifact.id, artifact_id, "artifact should match");
    drop(search);
    drop(blobs);
    drop(parser);
    drop(sqlite);

    let sqlite = SqliteStore::open(&layout.database_path)?;
    let events = EventLog::scan(&sqlite, EventFilter { artifact_id: None })?;
    let state = maestria_domain::replay_events(&events)?;
    let manifest = InstanceService::parse_manifest(&fs::read_to_string(&layout.manifest_path)?)?;
    drop(sqlite);

    let runtime = maestria_daemon::prepare_search_runtime(
        layout,
        &state,
        &manifest,
        RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
    )?;
    let (_, results) = runtime
        .execute("GraphRAG knowledge graphs".to_string(), 10)
        .await?;
    assert!(
        results
            .evidence
            .iter()
            .any(|candidate| candidate.evidence_id == evidence_id),
        "shared retrieval runtime should return the indexed evidence"
    );
    Ok(())
}

#[tokio::test]
async fn vertical_slice_init_index_search_evidence() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new()?;
    let mut env = prepare_test_env(&tmp).await?;

    let operation = async {
        let session = env
            .session
            .as_ref()
            .ok_or_else(|| std::io::Error::other("mutation session missing"))?;
        let IndexResult {
            evidence_id,
            first_chunk_id,
            event_count,
        } = index_and_verify_artifact(
            session,
            &env.layout.database_path,
            env.artifact_id,
            &env.hash,
            &env.bytes,
            &env.source_path,
        )
        .await?;

        attempt_idempotent_reindex(
            session,
            &env.layout.database_path,
            env.artifact_id,
            &env.hash,
            &env.bytes,
            &env.source_path,
            event_count,
        )
        .await?;
        Ok::<_, Box<dyn std::error::Error>>((evidence_id, first_chunk_id))
    }
    .await;

    let (evidence_id, first_chunk_id) = finish_session(env.session.take(), operation).await?;

    search_and_open_evidence_after_restart(
        &env.layout,
        env.artifact_id,
        evidence_id,
        first_chunk_id,
    )
    .await?;
    Ok(())
}

fn seed_stale_projections(
    layout: &InstanceLayout,
    artifact_id: ArtifactId,
) -> Result<RelationId, Box<dyn std::error::Error>> {
    let stale_relation_id = RelationId::new(999);
    let graph = SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))?;
    graph.insert_relation(Relation {
        id: stale_relation_id,
        source: RelationEndpoint::Artifact(artifact_id),
        kind: RelationKind::RelatedTo,
        target: RelationEndpoint::Card(CardId::new(999)),
        evidence_id: None,
        confidence_milli: 1,
        security: maestria_domain::SecurityMetadata::default(),
    })?;

    let vector = SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db"))?;
    vector.index_embeddings(vec![VectorEmbedding {
        chunk_id: ChunkId::new(999),
        vector: vec![1.0, 0.0],
        provenance: EmbeddingProvenance {
            content_hash: "stale".into(),
            identity: maestria_ports::contract_tests::fixture_embedding_identity("stale", 2)?,
            provider_id: "stale".into(),
            model: "stale".into(),
            model_version: "stale".into(),
            disclosure: maestria_ports::ProviderDisclosure {
                remote: false,
                retention: maestria_ports::RetentionPolicy::NoRetention,
            },
        },
    }])?;
    Ok(stale_relation_id)
}

fn verify_projections_rebuilt(
    layout: &InstanceLayout,
    artifact_id: ArtifactId,
    stale_relation_id: RelationId,
) -> Result<(), Box<dyn std::error::Error>> {
    let graph = SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))?;
    let query = GraphRelationQuery::new(RelationEndpoint::Artifact(artifact_id), u64::MAX)
        .ok_or("graph query limit must be positive")?;
    let relations = graph.get_relations_for(query)?.relations;
    assert!(
        relations
            .iter()
            .all(|relation| relation.id != stale_relation_id),
        "recovery should remove stale graph relations"
    );

    let vector = SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db"))?;
    let hits = vector.search_similar(VectorSearchQuery {
        vector: vec![1.0, 0.0],
        limit: 10,
        execution_budget: search_budget(10)?,
        ..Default::default()
    })?;
    assert!(
        hits.hits.is_empty(),
        "disabled embeddings should clear stale vectors"
    );
    Ok(())
}

#[tokio::test]
async fn vertical_slice_run_instance_restart_rebuilds_projections()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new()?;
    let mut env = prepare_test_env(&tmp).await?;
    let operation = async {
        let session = env
            .session
            .as_ref()
            .ok_or_else(|| std::io::Error::other("mutation session missing"))?;
        let IndexResult {
            evidence_id,
            first_chunk_id,
            ..
        } = index_and_verify_artifact(
            session,
            &env.layout.database_path,
            env.artifact_id,
            &env.hash,
            &env.bytes,
            &env.source_path,
        )
        .await?;
        Ok::<_, Box<dyn std::error::Error>>((evidence_id, first_chunk_id))
    }
    .await;

    let (evidence_id, first_chunk_id) = finish_session(env.session.take(), operation).await?;

    let stale_relation_id = seed_stale_projections(&env.layout, env.artifact_id)?;

    let shutdown = CancellationToken::new();
    let daemon = tokio::spawn(maestria_daemon::run_instance_with_shutdown(
        env.layout.root.clone(),
        shutdown.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;
    shutdown.cancel();
    daemon.await??;

    verify_projections_rebuilt(&env.layout, env.artifact_id, stale_relation_id)?;

    search_and_open_evidence_after_restart(
        &env.layout,
        env.artifact_id,
        evidence_id,
        first_chunk_id,
    )
    .await?;
    Ok(())
}

async fn wait_for_daemon_client(
    layout: &InstanceLayout,
) -> Result<maestria_daemon::DaemonClient, Box<dyn std::error::Error>> {
    for _ in 0..80 {
        if let Ok(client) = maestria_daemon::DaemonClient::from_instance(layout)
            && client
                .request(maestria_daemon::ClientOperation::Status)
                .await
                .is_ok()
        {
            return Ok(client);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err("daemon API did not become ready".into())
}

async fn wait_for_model_agent_terminal(
    client: &maestria_daemon::DaemonClient,
    layout: &InstanceLayout,
) -> Result<maestria_daemon::api::ModelAgentStatusResponse, Box<dyn std::error::Error>> {
    let mut approved = false;
    let mut last_status = "unobserved".to_string();
    for _ in 0..400 {
        let response = client
            .request(maestria_daemon::ClientOperation::ModelAgentStatus { run_id: 77 })
            .await?;
        if let maestria_daemon::ClientResponse::ModelAgentStatus(status) = response {
            last_status = status.status.clone();
            if matches!(status.status.as_str(), "succeeded" | "failed") {
                return Ok(status);
            }
            if !approved && let Some(approval_id) = status.approval_id {
                let generation = status
                    .journal_generation
                    .ok_or("pending approval omitted journal generation")?;
                let entries = SqliteStore::open(&layout.database_path)?.scan_in_flight()?;
                assert!(
                    entries.iter().any(|entry| {
                        entry.run_id.value() == 77 && entry.generation == generation
                    }),
                    "pending approval journal entry missing: {entries:?}"
                );
                client
                    .request(maestria_daemon::ClientOperation::ModelAgentResolve {
                        run_id: 77,
                        approval_id,
                        approved: true,
                    })
                    .await?;
                approved = true;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(format!(
        "model-agent proposal did not reach a terminal state; last status: {last_status}; approved: {approved}"
    )
    .into())
}

#[tokio::test]
async fn model_agent_proposal_round_trips_through_running_daemon()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new()?;
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes)?;
    let layout = setup_layout(tmp.path(), &notes)?;
    let shutdown = CancellationToken::new();
    let daemon = tokio::spawn(maestria_daemon::run_instance_with_shutdown(
        layout.root.clone(),
        shutdown.clone(),
    ));

    let client = wait_for_daemon_client(&layout).await?;

    let response = client
        .request(maestria_daemon::ClientOperation::ModelAgentPropose {
            proposal: maestria_daemon::api::ModelAgentProposalPayload {
                run_id: 77,
                task_id: None,
                query: "model agent smoke".into(),
                limit: 1,
                capability: "shell".into(),
                command: "echo model-agent-smoke".into(),
                working_directory: notes.display().to_string(),
                timeout_secs: 5,
                expected_generation: 1,
                evidence_ids: Vec::new(),
                task_validation: false,
                memory_candidate: false,
            },
        })
        .await?;

    match response {
        maestria_daemon::ClientResponse::ModelAgentProposal(result) => {
            assert_eq!(result.run_id, 77);
            assert_eq!(result.evidence_count, 0);
            assert!(result.harness.is_none());
            assert!(
                result
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("runtime accepted proposal correlation"))
            );
            assert!(result.validation.is_none());
            assert!(result.memory_candidate.is_none());
        }
        other => return Err(format!("unexpected model-agent response: {other:?}").into()),
    }

    let terminal = wait_for_model_agent_terminal(&client, &layout).await?;
    assert_eq!(
        terminal.status, "succeeded",
        "terminal proposal failed: {:?}",
        terminal.error
    );
    assert!(terminal.trace_id.is_some());
    assert_eq!(terminal.evidence_count, 0);
    let harness = terminal
        .harness
        .ok_or("terminal proposal omitted harness outcome")?;
    assert_eq!(harness.exit_code, 0);
    assert_eq!(harness.stdout.trim(), "model-agent-smoke");
    assert!(terminal.error.is_none());

    shutdown.cancel();
    daemon.await??;
    Ok(())
}
