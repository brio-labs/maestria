use std::{collections::BTreeSet, fs, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{InitInstanceInput, InstanceLayout, InstanceService};
use maestria_domain::{ArtifactId, DomainInput, KernelState, StartFullTextIndex, replay_events};
use maestria_governance::{AutonomyProfile, DefaultApprovalGate, DefaultRiskClassifier, Scope};
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_parsers::ParserRegistry;
use maestria_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EventFilter, EvidenceRepository,
    InMemoryHarnessAdapter,
};
use maestria_runtime::{Adapters, Governance, MaestriaRuntime, RuntimeConfig};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;
use maestria_web_evidence::UreqWebFetcher;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::info;
/// Collects distinct artifacts with pending full-text chunks and builds
/// `StartFullTextIndex` inputs so the runtime can resume indexing after
/// restart without re-parsing source bytes or re-playing `ParserCompleted`.
fn pending_start_full_text(state: &KernelState) -> Vec<DomainInput> {
    let mut artifacts: BTreeSet<ArtifactId> = BTreeSet::new();
    for chunk_id in &state.pending_full_text {
        if let Some(chunk) = state.chunks.get(chunk_id) {
            artifacts.insert(chunk.artifact_id);
        }
    }
    artifacts
        .into_iter()
        .map(|artifact_id| DomainInput::StartFullTextIndex(StartFullTextIndex { artifact_id }))
        .collect()
}

/// Reconcile projection repositories from replayed domain truth.
///
/// After `load_kernel_state` replays the event log into a `KernelState`,
/// this helper idempotently upserts every artifact, chunk, card, and
/// evidence from the replayed state into the SQLite projection tables.
///
/// Projection repair never emits domain events and never changes event
/// truth.  Startup recovery can then search/open evidence even if the
/// previous process crashed after event append but before a projection
/// write.
pub fn reconcile_projections(state: &KernelState, store: &SqliteStore) -> Result<()> {
    for artifact in state.artifacts.values() {
        ArtifactRepository::put(store, artifact.clone())
            .with_context(|| format!("put artifact {}", artifact.id))?;
    }
    for chunk in state.chunks.values() {
        ChunkRepository::put(store, chunk.clone())
            .with_context(|| format!("put chunk {}", chunk.id))?;
    }
    for card in state.cards.values() {
        CardRepository::put(store, card.clone())
            .with_context(|| format!("put card {}", card.id))?;
    }
    for evidence in state.evidences.values() {
        EvidenceRepository::put(store, evidence.clone())
            .with_context(|| format!("put evidence {}", evidence.id))?;
    }
    Ok(())
}

pub fn prepare_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    let plan = InstanceService::init_instance(InitInstanceInput { root: instance_dir })?;
    for directory in &plan.directories {
        fs::create_dir_all(directory)?;
    }
    if !plan.manifest_path.exists() {
        fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;
    }
    Ok(plan.layout)
}

pub fn load_kernel_state(layout: &InstanceLayout) -> Result<KernelState> {
    let sqlite_store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let events =
        maestria_ports::EventLog::scan(&sqlite_store, EventFilter { artifact_id: None })
            .with_context(|| format!("scan domain events {}", layout.database_path.display()))?;
    replay_events(&events).map_err(|error| anyhow!(error))
}

pub fn build_runtime(
    layout: &InstanceLayout,
    state: KernelState,
    profile: AutonomyProfile,
) -> Result<(
    MaestriaRuntime,
    mpsc::Sender<DomainInput>,
    mpsc::Receiver<DomainInput>,
    CancellationToken,
)> {
    let blob_store = Arc::new(
        FsBlobStore::open(&layout.blobs_dir)
            .with_context(|| format!("open blob store {}", layout.blobs_dir.display()))?,
    );
    let search_index = Arc::new(
        TantivyFullTextIndex::open(&layout.full_text_index_dir).with_context(|| {
            format!(
                "open full-text index {}",
                layout.full_text_index_dir.display()
            )
        })?,
    );
    let parser = Arc::new(ParserRegistry::with_defaults());
    let sqlite_store = Arc::new(
        SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?,
    );
    let event_log = sqlite_store.clone();
    let artifact_repo = sqlite_store.clone();
    let harness = Arc::new(InMemoryHarnessAdapter::default());
    let chunk_repo = sqlite_store.clone();
    let card_repo = sqlite_store.clone();
    let evidence_repo = sqlite_store.clone();
    let vector_index = Arc::new(
        SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db"))
            .with_context(|| format!("open vector index {}", layout.vector_index_dir.display()))?,
    );
    let graph_index = Arc::new(
        SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))
            .with_context(|| format!("open graph index {}", layout.graph_index_dir.display()))?,
    );
    let web_fetcher = Arc::new(UreqWebFetcher::new());

    let adapters = Adapters {
        event_log,
        blob_store,
        search_index,
        parser,
        harness,
        artifact_repo,
        chunk_repo,
        card_repo,
        evidence_repo,
        vector_index,
        graph_index,
        web_fetcher,
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let manifest_contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    let manifest = InstanceService::parse_manifest(&manifest_contents)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    let scope = Scope::new(
        manifest.read_roots,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        false,
    );
    let config = RuntimeConfig {
        profile,
        scope,
        ..Default::default()
    };

    let shutdown_token = CancellationToken::new();
    let (runtime, input_rx) = MaestriaRuntime::new(config, state, adapters, governance);
    let input_tx = runtime.handle().input_tx.clone();
    Ok((runtime, input_tx, input_rx, shutdown_token))
}

pub async fn run_instance(instance_dir: PathBuf) -> Result<()> {
    let layout = prepare_instance(instance_dir).with_context(|| "prepare instance layout")?;
    let state = load_kernel_state(&layout).with_context(|| "load persisted kernel state")?;

    // Repair projection repositories before runtime start so that
    // artifact, chunk, card, and evidence lookups succeed even if the
    // previous process crashed after event append but before a
    // projection write.
    {
        let store = SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
        reconcile_projections(&state, &store)
            .with_context(|| "reconcile projection repositories")?;
    }

    // Compute recovery inputs before state is moved into build_runtime.
    let pending = pending_start_full_text(&state);

    let (runtime, input_tx, input_rx, shutdown_token) =
        build_runtime(&layout, state, AutonomyProfile::ReadOnly)?;

    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    // Submit pending StartFullTextIndex inputs after the runtime task has
    // started consuming from `input_rx` to avoid bounded-channel deadlock.
    for input in pending {
        input_tx
            .send(input)
            .await
            .map_err(|e| anyhow!("failed to queue restart full-text index: {e}"))?;
    }

    let root = layout.root.clone();
    info!(root = %root.display(), "runtime started");

    tokio::signal::ctrl_c().await?;
    info!(root = %root.display(), "shutdown requested");
    shutdown_token.cancel();

    runtime_task
        .await
        .with_context(|| "runtime loop join failed")?;

    Ok(())
}
#[cfg(test)]
mod projection_recovery_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{
        ArtifactDetected, ChunkId, MaestriaEffect, ParserResult, RegisterChunkInput,
    };

    #[test]
    fn pending_start_full_text_groups_by_artifact() {
        let mut state = KernelState::new();
        let artifact_id = ArtifactId::new(1);

        state
            .apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
                artifact_id,
                title: "test.md".to_string(),
                source_path: "/tmp/test.md".to_string(),
                source_bytes: vec![1, 2, 3],
                content_hash: "sha256:abc".to_string(),
            }))
            .expect("register artifact");

        state
            .apply_input(DomainInput::ParserCompleted(ParserResult {
                artifact_id,
                chunks: vec![
                    RegisterChunkInput {
                        chunk_id: ChunkId::new(10),
                        artifact_id,
                        order: 0,
                        text: "chunk a".to_string(),
                    },
                    RegisterChunkInput {
                        chunk_id: ChunkId::new(11),
                        artifact_id,
                        order: 1,
                        text: "chunk b".to_string(),
                    },
                ],
                cards: Vec::new(),
            }))
            .expect("parser completed");

        assert_eq!(state.pending_full_text.len(), 2);

        let inputs = pending_start_full_text(&state);
        assert_eq!(
            inputs.len(),
            1,
            "should produce one StartFullTextIndex input per artifact"
        );

        match &inputs[0] {
            DomainInput::StartFullTextIndex(start) => {
                assert_eq!(start.artifact_id, artifact_id);
            }
            other => panic!("expected StartFullTextIndex, got {other:?}"),
        }
    }

    #[test]
    fn pending_start_full_text_resumes_indexing_without_reparse() {
        // Simulate a restart scenario: chunks were created (persisted) but
        // full-text indexing wasn't completed before shutdown. On restart,
        // pending_start_full_text produces StartFullTextIndex inputs that
        // emit IndexFullText effects without re-parsing source bytes.

        let mut state = KernelState::new();
        let artifact_id = ArtifactId::new(1);

        state
            .apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
                artifact_id,
                title: "notes.md".to_string(),
                source_path: String::new(),
                source_bytes: vec![1, 2, 3],
                content_hash: "sha256:def".to_string(),
            }))
            .expect("register artifact");

        let output = state
            .apply_input(DomainInput::ParserCompleted(ParserResult {
                artifact_id,
                chunks: vec![
                    RegisterChunkInput {
                        chunk_id: ChunkId::new(20),
                        artifact_id,
                        order: 0,
                        text: "hello".to_string(),
                    },
                    RegisterChunkInput {
                        chunk_id: ChunkId::new(21),
                        artifact_id,
                        order: 1,
                        text: "world".to_string(),
                    },
                ],
                cards: Vec::new(),
            }))
            .expect("parser completed");

        assert_eq!(state.pending_full_text.len(), 2);
        // ParserCompleted no longer emits IndexFullText effects; indexing is
        // deferred to StartFullTextIndex.
        let parser_index_effects: Vec<_> = output
            .effects
            .iter()
            .filter(|e| matches!(e, MaestriaEffect::IndexFullText(_)))
            .collect();
        assert!(
            parser_index_effects.is_empty(),
            "ParserCompleted must not emit IndexFullText effects"
        );

        let event_count_before = state.event_log.len();

        // Simulate restart: build pending inputs and apply to the same state
        let pending_inputs = pending_start_full_text(&state);
        assert_eq!(pending_inputs.len(), 1);

        let restart_output = state
            .apply_input(pending_inputs[0].clone())
            .expect("restart start full-text index should succeed");

        // StartFullTextIndex emits IndexFullText effects but no new events.
        let event_count_after = state.event_log.len();
        assert_eq!(
            event_count_after, event_count_before,
            "StartFullTextIndex must not produce duplicate events"
        );

        let restart_index_effects: Vec<_> = restart_output
            .effects
            .iter()
            .filter(|e| matches!(e, MaestriaEffect::IndexFullText(_)))
            .collect();
        assert_eq!(
            restart_index_effects.len(),
            2,
            "StartFullTextIndex should emit IndexFullText for both pending chunks"
        );

        assert_eq!(state.pending_full_text.len(), 2);
    }

    #[test]
    fn pending_start_full_text_empty_when_nothing_pending() {
        let state = KernelState::new();
        let inputs = pending_start_full_text(&state);
        assert!(inputs.is_empty());
    }

    #[test]
    fn pending_start_full_text_skips_orphan_chunk_ids() {
        // If pending_full_text references a chunk_id not in state.chunks,
        // the helper should silently skip it.
        let mut state = KernelState::new();
        state.pending_full_text.insert(ChunkId::new(999));

        let inputs = pending_start_full_text(&state);
        assert!(inputs.is_empty(), "orphan chunk ids should be skipped");
    }
}
