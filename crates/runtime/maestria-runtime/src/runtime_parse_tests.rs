use super::*;
use maestria_domain::{
    Artifact, ArtifactId, EvidenceKind, IndexStatus, LogicalTick, ParseArtifactRequest,
};
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier};
use maestria_ports::{
    FileHandle, FileMetadata, InMemoryArtifactRepository, InMemoryBlobStore,
    InMemoryCardRepository, InMemoryChunkRepository, InMemoryEventLog,
    InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryGraphIndex,
    InMemoryHarnessAdapter, InMemoryParser, InMemoryVectorIndex, InMemoryWebFetcher,
    ParseContext, ParsedArtifact, Parser, PortError,
};
use std::collections::BTreeSet;
use std::sync::Arc;


#[tokio::test]
async fn parse_artifact_passes_exact_source_path_and_bytes() {
    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: ArtifactId::new(42),
            title: "artifact-title-unused".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
        })
        .expect("pre-populated artifact should be accepted");

    let adapters = Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(artifact_repo),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(42),
            source_path: "/repo/src/main.rs".to_string(),
            source_bytes: b"fn hello() {}".to_vec(),
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    )
    .await;

    assert!(result, "ParseArtifact should succeed");

    // First input: ParserCompleted (sent before evidence so the domain can commit the artifact).
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserCompleted(pr))) => {
            assert_eq!(pr.artifact_id, ArtifactId::new(42));
            assert_eq!(pr.chunks.len(), 1);
            assert_eq!(pr.chunks[0].text, "fn hello() {}");
        }
        Ok(Some(other)) => panic!("expected ParserCompleted, got {other:?}"),
        Ok(None) => panic!("channel closed before ParserCompleted"),
        Err(_) => panic!("timeout waiting for ParserCompleted"),
    }

    // Second input: RecordEvidence for the single parsed chunk
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::RecordEvidence(ev))) => {
            assert_eq!(ev.artifact_id, ArtifactId::new(42));
            assert_eq!(ev.claim_id, None);
            assert_eq!(ev.observed_at, LogicalTick::new(1));
            match &ev.kind {
                EvidenceKind::FileSpan {
                    path,
                    range,
                    content_hash,
                    snapshot,
                } => {
                    assert_eq!(path, "/repo/src/main.rs");
                    assert_eq!(range.start, 1);
                    assert_eq!(range.end, 1);
                    assert!(content_hash.starts_with("sha256:"));
                    assert!(snapshot.is_some());
                }
                _ => panic!("expected FileSpan evidence, got {:?}", ev.kind),
            }
            assert!(!ev.excerpt.is_empty());
        }
        Ok(Some(other)) => panic!("expected RecordEvidence, got {other:?}"),
        Ok(None) => panic!("channel closed before RecordEvidence"),
        Err(_) => panic!("timeout waiting for RecordEvidence"),
    }
}

#[tokio::test]
async fn parse_artifact_empty_bytes_returns_failure() {
    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: ArtifactId::new(7),
            title: "unused".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
        })
        .expect("pre-populated artifact should be accepted");

    let adapters = Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(artifact_repo),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let (input_tx, _input_rx) = mpsc::channel(8);

    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(7),
            source_path: "/repo/empty.rs".to_string(),
            source_bytes: Vec::new(),
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    )
    .await;

    assert!(!result, "ParseArtifact with empty bytes should fail");
}

#[tokio::test]
async fn parse_artifact_unsupported_parser_returns_failure() {
    struct RejectingParser;
    impl Parser for RejectingParser {
        fn id(&self) -> &'static str {
            "rejecting"
        }
        fn supports(&self, _file: &FileMetadata) -> bool {
            false
        }
        fn parse(
            &self,
            _file: FileHandle,
            _context: ParseContext,
        ) -> Result<ParsedArtifact, PortError> {
            Err(PortError::InvalidInput {
                message: "never called".into(),
            })
        }
    }

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: ArtifactId::new(9),
            title: "unsupported".into(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
        })
        .expect("pre-populated artifact should be accepted");

    let adapters = Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(RejectingParser),
        artifact_repo: Arc::new(artifact_repo),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let (input_tx, _input_rx) = mpsc::channel(8);

    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(9),
            source_path: "/repo/data.pdf".to_string(),
            source_bytes: b"pdf content".to_vec(),
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    )
    .await;

    assert!(
        !result,
        "unsupported parser should cause ParseArtifact to return false for retry"
    );
}

#[tokio::test]
async fn parse_artifact_staged_ingestion_constructs_ephemeral_context() {
    // No artifact in repo or state — staged ingestion path.
    let adapters = Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let artifact_id = ArtifactId::new(99);
    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id,
            source_path: "/repo/ghost.rs".to_string(),
            source_bytes: b"fn gone() {}".to_vec(),
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    )
    .await;

    assert!(
        result,
        "staged ParseArtifact should succeed with ephemeral context"
    );

    // First input: ParserCompleted (sent before evidence so domain commits the artifact).
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserCompleted(pr))) => {
            assert_eq!(pr.artifact_id, artifact_id);
            assert_eq!(pr.chunks.len(), 1);
        }
        Ok(Some(other)) => panic!("expected ParserCompleted, got {other:?}"),
        Ok(None) => panic!("channel closed before ParserCompleted"),
        Err(_) => panic!("timeout waiting for ParserCompleted"),
    }

    // Second input: RecordEvidence for the parsed chunk
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::RecordEvidence(ev))) => {
            assert_eq!(ev.artifact_id, artifact_id);
            assert_eq!(ev.claim_id, None);
            match &ev.kind {
                EvidenceKind::FileSpan { path, snapshot, .. } => {
                    assert_eq!(path, "/repo/ghost.rs");
                    assert!(snapshot.is_some(), "evidence must carry a blob snapshot");
                }
                _ => panic!("expected FileSpan evidence, got {:?}", ev.kind),
            }
        }
        Ok(Some(other)) => panic!("expected RecordEvidence, got {other:?}"),
        Ok(None) => panic!("channel closed before RecordEvidence"),
        Err(_) => panic!("timeout waiting for RecordEvidence"),
    }
}

#[tokio::test]
async fn parse_artifact_repository_error_returns_failure() {
    // Repository errors remain retryable — they must not be swallowed.
    struct FailingArtifactRepo;
    impl ArtifactRepository for FailingArtifactRepo {
        fn get(&self, _id: ArtifactId) -> Result<Option<Artifact>, PortError> {
            Err(PortError::Downstream {
                message: "db unavailable".to_string(),
            })
        }
        fn put(&self, _artifact: Artifact) -> Result<(), PortError> {
            Ok(())
        }
    }

    let adapters = Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(FailingArtifactRepo),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let (input_tx, _input_rx) = mpsc::channel(8);

    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(99),
            source_path: "/repo/ghost.rs".to_string(),
            source_bytes: b"fn gone() {}".to_vec(),
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    )
    .await;

    assert!(
        !result,
        "repository error should return false so retry policy remains active"
    );
}

