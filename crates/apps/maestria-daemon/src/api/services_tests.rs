use super::*;
use maestria_domain::{ArtifactId, EvidenceId, HarnessRunId};
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier, DefaultValidationGate};
use maestria_ports::{
    HarnessCommandClass, HarnessRequest, InMemoryApprovalRepository, InMemoryArtifactRepository,
    InMemoryBlobStore, InMemoryCardRepository, InMemoryChunkRepository, InMemoryEffectJournal,
    InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryGraphIndex,
    InMemoryHarnessAdapter, InMemoryIdAllocator, InMemoryParser, InMemoryVectorIndex,
    InMemoryWebFetcher,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

fn test_adapters() -> maestria_runtime::Adapters {
    maestria_runtime::Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        embedding_provider: None,
        search_executor: None,
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
        id_allocator: Arc::new(InMemoryIdAllocator::new()),
        effect_journal: Arc::new(InMemoryEffectJournal::default()),
        approval_repo: Arc::new(InMemoryApprovalRepository::new()),
    }
}

fn test_governance() -> maestria_runtime::Governance {
    maestria_runtime::Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)),
        memory_promotion_gate: Arc::new(maestria_governance::DefaultMemoryPromotionGate),
    }
}

struct TempDir(PathBuf);

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

impl TempDir {
    fn create() -> std::io::Result<Self> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "maestria-services-test-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    _temp_dir: TempDir,
    context: ApiContext,
    input_rx: mpsc::Receiver<DomainInput>,
    layout: InstanceLayout,
}

fn fixture(buffer: usize) -> Result<Fixture> {
    let temp_dir = TempDir::create()?;
    let root = temp_dir.path().to_path_buf();
    let layout = InstanceLayout::for_root(root.clone());
    std::fs::create_dir_all(&layout.system_dir)?;
    if let Some(parent) = layout.database_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manifest = InstanceManifest::default_for_root(root);
    std::fs::write(&layout.manifest_path, manifest.encode())?;

    let (input_tx, input_rx) = mpsc::channel(buffer);
    let context = ApiContext {
        layout: layout.clone(),
        token: "test-token".to_string(),
        socket_path: layout.system_dir.join("test.sock"),
        input_tx,
        adapters: Arc::new(test_adapters()),
        governance: Arc::new(test_governance()),
    };
    Ok(Fixture {
        _temp_dir: temp_dir,
        context,
        input_rx,
        layout,
    })
}

fn proposal() -> ModelAgentProposal {
    ModelAgentProposal {
        run_id: HarnessRunId::new(1),
        task_id: None,
        query: "test".to_string(),
        limit: 10,
        capability: "shell".to_string(),
        command: "echo hello".to_string(),
        working_directory: PathBuf::from("."),
        timeout: Duration::from_secs(30),
        expected_generation: 0,
        evidence_ids: Vec::new(),
    }
}

fn governed(root: PathBuf, evidence_ids: Vec<EvidenceId>) -> maestria_ports::GovernedAgentProposal {
    maestria_ports::GovernedAgentProposal {
        search_query: String::new(),
        search_limit: 10,
        evidence_ids,
        harness: HarnessRequest {
            run_id: HarnessRunId::new(1),
            command: "echo hello".to_string(),
            working_directory: root,
            duration_budget: Duration::from_secs(30),
            class: HarnessCommandClass::Shell,
            readable_roots: Vec::new(),
            blocked_paths: Vec::new(),
            blocked_patterns: Vec::new(),
        },
    }
}

#[tokio::test]
async fn harness_completion_waits_for_bounded_delivery() -> Result<()> {
    let mut fixture = fixture(1)?;
    fixture
        .context
        .input_tx
        .send(DomainInput::ClockTick(maestria_domain::LogicalTick::new(1)))
        .await?;
    let proposal = proposal();
    let governed = governed(fixture.layout.root.clone(), Vec::new());
    let state = KernelState::new();
    let context = fixture.context;
    let handle = tokio::spawn(async move {
        execute_proposal_effects(&context, &proposal, &governed, &state).await
    });

    let filler = fixture.input_rx.recv().await;
    assert!(matches!(filler, Some(DomainInput::ClockTick(_))));
    let effects = tokio::time::timeout(Duration::from_secs(2), handle).await??;
    assert!(effects.harness.is_some());
    assert!(
        effects.warnings.is_empty(),
        "unexpected warnings: {:?}",
        effects.warnings
    );
    assert!(matches!(
        fixture.input_rx.recv().await,
        Some(DomainInput::HarnessRunCompleted(_))
    ));
    Ok(())
}

#[tokio::test]
async fn closed_harness_channel_preserves_delivery_failure() -> Result<()> {
    let fixture = fixture(1)?;
    let input_rx = fixture.input_rx;
    drop(input_rx);
    let governed = governed(fixture.layout.root.clone(), Vec::new());
    let effects = execute_proposal_effects(
        &fixture.context,
        &proposal(),
        &governed,
        &KernelState::new(),
    )
    .await;
    assert!(effects.harness.is_none());
    assert!(
        effects
            .warnings
            .iter()
            .any(|warning| warning.contains("channel closed"))
    );
    Ok(())
}

#[tokio::test]
async fn harness_scope_and_journal_are_enforced() -> Result<()> {
    let fixture = fixture(8)?;
    let outside = governed(PathBuf::from("/"), Vec::new());
    let result = execute_governed_harness(&fixture.context, &proposal(), &outside).await;
    assert!(result.is_err());

    let inside = governed(fixture.layout.root.clone(), Vec::new());
    let (outcome, generation) =
        execute_governed_harness(&fixture.context, &proposal(), &inside).await?;
    assert_eq!(outcome.exit_code, 0);
    assert!(
        fixture
            .context
            .adapters
            .effect_journal
            .is_feedback_accepted(inside.harness.run_id, generation)?
    );
    Ok(())
}

#[tokio::test]
async fn memory_candidate_waits_for_bounded_delivery() -> Result<()> {
    let mut fixture = fixture(1)?;
    fixture
        .context
        .input_tx
        .send(DomainInput::ClockTick(maestria_domain::LogicalTick::new(1)))
        .await?;
    let evidence_id = EvidenceId::new(1);
    let mut state = KernelState::new();
    state.evidences.insert(
        evidence_id,
        maestria_domain::Evidence {
            id: evidence_id,
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: maestria_domain::EvidenceKind::FileSpan {
                path: "test.md".to_string(),
                range: maestria_domain::ContentRange { start: 0, end: 1 },
                content_hash: "sha256:test".to_string(),
                snapshot: None,
            },
            excerpt: "test".to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: maestria_domain::SecurityMetadata::default(),
        },
    );
    let harness = Some(ModelAgentHarnessOutcome {
        exit_code: 0,
        stdout: "ok".to_string(),
        stderr: String::new(),
        duration_ms: 1,
    });
    let governed = governed(fixture.layout.root.clone(), vec![evidence_id]);
    let context = fixture.context;
    let handle = tokio::spawn(async move {
        create_memory_candidate(&context, &governed, &state, &harness).await
    });
    assert!(matches!(
        fixture.input_rx.recv().await,
        Some(DomainInput::ClockTick(_))
    ));
    let (candidate, warning) = tokio::time::timeout(Duration::from_secs(2), handle).await??;
    assert!(candidate.is_some());
    assert!(warning.is_none(), "unexpected warning: {warning:?}");
    assert!(matches!(
        fixture.input_rx.recv().await,
        Some(DomainInput::CreateMemoryCandidate(_))
    ));
    Ok(())
}
