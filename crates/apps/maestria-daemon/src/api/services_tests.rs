use super::*;
use maestria_domain::{
    Artifact, ArtifactId, Evidence, EvidenceId, EvidenceKind, IndexStatus, ScopeId,
    SecurityMetadata, ValidationReportId,
};
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier, DefaultValidationGate};
use maestria_ports::{
    ArtifactRepository, EvidenceRepository, HarnessAdapter, HarnessCapabilities,
    HarnessCommandClass, HarnessOutcome, HarnessRequest, InMemoryApprovalRepository,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository, InMemoryChunkRepository,
    InMemoryEffectJournal, InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex,
    InMemoryGraphIndex, InMemoryHarnessAdapter, InMemoryIdAllocator, InMemoryParser,
    InMemoryVectorIndex, InMemoryWebFetcher, PortError,
};
use maestria_storage_sqlite::SqliteStore;
use std::collections::BTreeSet;
use std::future::Future;
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

struct FailingHarnessAdapter;

impl HarnessAdapter for FailingHarnessAdapter {
    fn capabilities(&self) -> std::result::Result<HarnessCapabilities, PortError> {
        Ok(HarnessCapabilities {
            command_classes: vec![HarnessCommandClass::Shell],
            write_enabled: false,
            read_enabled: true,
            web_enabled: false,
        })
    }

    fn execute(
        &self,
        _request: HarnessRequest,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = std::result::Result<HarnessOutcome, PortError>> + Send + '_>,
    > {
        Box::pin(std::future::ready(Err(PortError::Internal {
            message: "simulated harness failure".to_string(),
        })))
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
    let effects = tokio::time::timeout(Duration::from_secs(2), handle).await???;
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
async fn closed_harness_channel_returns_typed_failure_and_pauses_journal() -> Result<()> {
    let fixture = fixture(1)?;
    let input_rx = fixture.input_rx;
    drop(input_rx);
    let run_id = proposal().run_id;
    let governed = governed(fixture.layout.root.clone(), Vec::new());
    let failure = match execute_proposal_effects(
        &fixture.context,
        &proposal(),
        &governed,
        &KernelState::new(),
    )
    .await
    {
        Ok(_) => return Err(anyhow::anyhow!("closed channel unexpectedly succeeded")),
        Err(failure) => failure,
    };
    assert!(matches!(
        failure,
        ProposalEffectFailure::CompletionDelivery { .. }
    ));
    assert!(
        fixture
            .context
            .adapters
            .effect_journal
            .scan_in_flight()?
            .is_empty()
    );
    assert!(
        !fixture
            .context
            .adapters
            .effect_journal
            .is_current(run_id, 1)?
    );
    Ok(())
}

#[tokio::test]
async fn harness_execution_failure_returns_typed_failure_and_terminalizes_journal() -> Result<()> {
    let fixture = fixture(8)?;
    let mut context = fixture.context;
    let mut adapters = test_adapters();
    adapters.harness = Arc::new(FailingHarnessAdapter);
    context.adapters = Arc::new(adapters);
    let run_id = proposal().run_id;
    let governed = governed(fixture.layout.root.clone(), Vec::new());
    let failure =
        match execute_proposal_effects(&context, &proposal(), &governed, &KernelState::new()).await
        {
            Ok(_) => return Err(anyhow::anyhow!("failing harness unexpectedly succeeded")),
            Err(failure) => failure,
        };
    assert!(matches!(failure, ProposalEffectFailure::Harness { .. }));
    assert!(context.adapters.effect_journal.scan_in_flight()?.is_empty());
    assert!(!context.adapters.effect_journal.is_current(run_id, 1)?);
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

#[test]
fn harness_scope_keeps_runtime_privacy_patterns_when_manifest_omits_pem() -> Result<()> {
    let fixture = fixture(8)?;
    let mut manifest = InstanceManifest::default_for_root(fixture.layout.root.clone());
    manifest
        .excluded_patterns
        .retain(|pattern| pattern != "*.pem");
    std::fs::write(&fixture.layout.manifest_path, manifest.encode())?;

    let scope = harness_scope(&fixture.context, &fixture.layout.root)?;
    assert!(
        scope
            .blocked_patterns()
            .iter()
            .any(|pattern| pattern == ".ssh"),
        "manifest .ssh exclusion was not forwarded to the harness scope"
    );
    assert!(
        scope
            .blocked_patterns()
            .iter()
            .any(|pattern| pattern == "*.pem"),
        "default pem privacy exclusion was not forwarded to the harness scope"
    );
    assert!(
        scope
            .blocked_patterns()
            .iter()
            .any(|pattern| pattern == "password"),
        "default sensitive-name privacy exclusion was not forwarded to the harness scope"
    );
    let pem_path = fixture
        .layout
        .root
        .join("private.pem")
        .display()
        .to_string();
    assert!(
        !source_scope_allowed(&manifest, &pem_path),
        "default pem privacy exclusion did not protect evidence access"
    );
    let ssh_path = fixture.layout.root.join(".ssh").join("id_rsa");
    let ssh_path = ssh_path.display().to_string();
    assert!(
        !source_scope_allowed(&manifest, &ssh_path),
        "manifest .ssh exclusion did not protect evidence access"
    );
    Ok(())
}

#[test]
fn open_evidence_rejects_file_span_outside_current_manifest_roots() -> Result<()> {
    let fixture = fixture(8)?;
    let store = SqliteStore::open(&fixture.layout.database_path)?;
    let artifact_id = ArtifactId::new(41);
    let evidence_id = EvidenceId::new(42);
    ArtifactRepository::put(
        &store,
        Artifact {
            id: artifact_id,
            title: "outside.md".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Indexed,
            content_hash: Some("hash".to_string()),
            parse_status: None,
            security: maestria_domain::SecurityMetadata::default(),
        },
    )?;
    let outside_path = fixture.layout.root.join("..").join("indexed-outside.md");
    EvidenceRepository::put(
        &store,
        Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: outside_path.display().to_string(),
                range: maestria_domain::ContentRange { start: 1, end: 1 },
                content_hash: "hash".to_string(),
                snapshot: None,
            },
            excerpt: "outside".to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: maestria_domain::SecurityMetadata::default(),
        },
    )?;
    drop(store);

    let error = match open_evidence(&fixture.layout, evidence_id.value()) {
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "out-of-scope indexed evidence unexpectedly opened"
            ));
        }
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("outside instance read roots or excluded by policy"),
        "unexpected out-of-scope evidence error: {error:#}"
    );
    Ok(())
}

#[test]
fn open_evidence_rejects_indexed_non_file_evidence_from_other_scope() -> Result<()> {
    let fixture = fixture(8)?;
    let store = SqliteStore::open(&fixture.layout.database_path)?;
    let artifact_id = ArtifactId::new(51);
    let evidence_id = EvidenceId::new(52);
    ArtifactRepository::put(
        &store,
        Artifact {
            id: artifact_id,
            title: "validation-report".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Indexed,
            content_hash: None,
            parse_status: None,
            security: SecurityMetadata {
                scope_id: Some(ScopeId::new(1)),
                ..SecurityMetadata::default()
            },
        },
    )?;
    EvidenceRepository::put(
        &store,
        Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::Validation {
                report_id: ValidationReportId::new(7),
            },
            excerpt: "validation report".to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: SecurityMetadata {
                scope_id: Some(ScopeId::new(99)),
                ..SecurityMetadata::default()
            },
        },
    )?;
    drop(store);

    let error = match open_evidence(&fixture.layout, evidence_id.value()) {
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "cross-instance indexed non-file evidence unexpectedly opened"
            ));
        }
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("evidence is not available: not available under retrieval policy"),
        "unexpected cross-instance evidence error: {error:#}"
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
        create_memory_candidate(&context, &governed, &state, &harness, None).await
    });
    assert!(matches!(
        fixture.input_rx.recv().await,
        Some(DomainInput::ClockTick(_))
    ));
    let candidate = tokio::time::timeout(Duration::from_secs(2), handle).await???;
    assert!(candidate.is_some());
    assert!(matches!(
        fixture.input_rx.recv().await,
        Some(DomainInput::CreateMemoryCandidate(_))
    ));
    Ok(())
}

#[tokio::test]
async fn search_failure_returns_typed_failure_without_starting_harness() -> Result<()> {
    let fixture = fixture(8)?;
    let mut governed = governed(fixture.layout.root.clone(), Vec::new());
    governed.search_query = "missing search index".to_string();
    let failure = match execute_proposal_effects(
        &fixture.context,
        &proposal(),
        &governed,
        &KernelState::new(),
    )
    .await
    {
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "missing search index unexpectedly succeeded"
            ));
        }
        Err(failure) => failure,
    };
    assert!(matches!(failure, ProposalEffectFailure::Search { .. }));
    assert!(
        fixture
            .context
            .adapters
            .effect_journal
            .scan_in_flight()?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn candidate_delivery_failure_returns_typed_failure() -> Result<()> {
    let fixture = fixture(1)?;
    let input_rx = fixture.input_rx;
    drop(input_rx);
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
    let failure =
        match create_memory_candidate(&fixture.context, &governed, &state, &harness, None).await {
            Ok(_) => return Err(anyhow::anyhow!("closed channel unexpectedly succeeded")),
            Err(failure) => failure,
        };
    assert!(matches!(
        failure,
        ProposalEffectFailure::CandidateDelivery { .. }
    ));
    Ok(())
}

#[test]
fn truncate_utf8_keeps_multibyte_output_on_character_boundaries() {
    let truncated = truncate_utf8("😀😀😀".as_bytes(), 7);
    assert_eq!(truncated, "😀...");
    assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
}

#[test]
fn truncate_utf8_handles_repeated_unicode_over_limit() {
    let truncated = truncate_utf8("界".repeat(128).as_bytes(), 10);
    assert_eq!(truncated, "界界...");
    assert!(truncated.len() <= 10);
    assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
}
