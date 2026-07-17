use super::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, EvidenceKind, IndexStatus, LogicalTick, ParseArtifactRequest,
};
use maestria_ports::{
    ArtifactRepository, FileHandle, FileMetadata, InMemoryArtifactRepository, ParseContext,
    ParsedArtifact, Parser, PortError,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

#[tokio::test]
async fn parse_artifact_passes_exact_source_path_and_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo.put(Artifact {
        id: ArtifactId::new(42),
        title: "artifact-title-unused".to_string(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::Unindexed,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    })?;

    let adapters = Adapters {
        artifact_repo: Arc::new(artifact_repo),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(42),
            source_path: "/repo/src/main.rs".to_string(),
            source_bytes: b"fn hello() {}".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(result, "ParseArtifact should succeed");

    // First input: ParserStarted (sent before parsing so crash-recovery can resume).
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserStarted(ps))) => {
            assert_eq!(ps.artifact_id, ArtifactId::new(42));
            assert_eq!(ps.source_path, "/repo/src/main.rs");
            assert!(!ps.content_hash.is_empty());
            assert!(ps.blob_id.value() > 0);
        }
        Ok(Some(other)) => return Err(format!("expected ParserStarted, got {other:?}").into()),
        Ok(None) => return Err("channel closed before ParserStarted".to_string().into()),
        Err(_) => return Err("timeout waiting for ParserStarted".to_string().into()),
    }

    // Second input: ParserCompleted (sent before evidence so the domain can commit the artifact).
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserCompleted(pr))) => {
            assert_eq!(pr.artifact_id, ArtifactId::new(42));
            assert_eq!(pr.chunks.len(), 1);
            assert_eq!(pr.chunks[0].text, "fn hello() {}");
        }
        Ok(Some(other)) => return Err(format!("expected ParserCompleted, got {other:?}").into()),
        Ok(None) => return Err("channel closed before ParserCompleted".to_string().into()),
        Err(_) => return Err("timeout waiting for ParserCompleted".to_string().into()),
    }

    // Third input: RecordEvidence for the single parsed chunk
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
                _ => return Err(format!("expected FileSpan evidence, got {:?}", ev.kind).into()),
            }
            assert!(!ev.excerpt.is_empty());
        }
        Ok(Some(other)) => return Err(format!("expected RecordEvidence, got {other:?}").into()),
        Ok(None) => return Err("channel closed before RecordEvidence".to_string().into()),
        Err(_) => return Err("timeout waiting for RecordEvidence".to_string().into()),
    }
    Ok(())
}

#[tokio::test]
async fn parse_artifact_empty_bytes_returns_failure() -> Result<(), Box<dyn std::error::Error>> {
    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo.put(Artifact {
        id: ArtifactId::new(7),
        title: "unused".to_string(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::Unindexed,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    })?;
    let adapters = Adapters {
        artifact_repo: Arc::new(artifact_repo),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(7),
            source_path: "/repo/empty.rs".to_string(),
            source_bytes: Vec::new(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(!result, "ParseArtifact with empty bytes should fail");
    Ok(())
}
#[tokio::test]
async fn parse_artifact_unsupported_parser_returns_failure()
-> Result<(), Box<dyn std::error::Error>> {
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
    artifact_repo.put(Artifact {
        id: ArtifactId::new(9),
        title: "unsupported".into(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::Unindexed,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    })?;
    let adapters = Adapters {
        parser: Arc::new(RejectingParser),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(9),
            source_path: "/repo/data.pdf".to_string(),
            source_bytes: b"pdf content".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(
        !result,
        "unsupported parser should cause ParseArtifact to return false for retry"
    );
    Ok(())
}

#[tokio::test]
async fn parse_artifact_staged_ingestion_constructs_ephemeral_context()
-> Result<(), Box<dyn std::error::Error>> {
    // No artifact in repo or state — staged ingestion path.
    let adapters = crate::test_helpers::test_adapters();
    let governance = crate::test_helpers::test_governance();
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let artifact_id = ArtifactId::new(99);
    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id,
            source_path: "/repo/ghost.rs".to_string(),
            source_bytes: b"fn gone() {}".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(
        result,
        "staged ParseArtifact should succeed with ephemeral context"
    );

    // First input: ParserStarted (sent before parsing).
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserStarted(ps))) => {
            assert_eq!(ps.artifact_id, artifact_id);
            assert_eq!(ps.source_path, "/repo/ghost.rs");
        }
        Ok(Some(other)) => return Err(format!("expected ParserStarted, got {other:?}").into()),
        Ok(None) => return Err("channel closed before ParserStarted".to_string().into()),
        Err(_) => return Err("timeout waiting for ParserStarted".to_string().into()),
    }

    // Second input: ParserCompleted (sent before evidence so domain commits the artifact).
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserCompleted(pr))) => {
            assert_eq!(pr.artifact_id, artifact_id);
            assert_eq!(pr.chunks.len(), 1);
        }
        Ok(Some(other)) => return Err(format!("expected ParserCompleted, got {other:?}").into()),
        Ok(None) => return Err("channel closed before ParserCompleted".to_string().into()),
        Err(_) => return Err("timeout waiting for ParserCompleted".to_string().into()),
    }

    // Third input: RecordEvidence for the parsed chunk
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::RecordEvidence(ev))) => {
            assert_eq!(ev.artifact_id, artifact_id);
            assert_eq!(ev.claim_id, None);
            match &ev.kind {
                EvidenceKind::FileSpan { path, snapshot, .. } => {
                    assert_eq!(path, "/repo/ghost.rs");
                    assert!(snapshot.is_some(), "evidence must carry a blob snapshot");
                }
                _ => return Err(format!("expected FileSpan evidence, got {:?}", ev.kind).into()),
            }
        }
        Ok(Some(other)) => return Err(format!("expected RecordEvidence, got {other:?}").into()),
        Ok(None) => return Err("channel closed before RecordEvidence".to_string().into()),
        Err(_) => return Err("timeout waiting for RecordEvidence".to_string().into()),
    }
    Ok(())
}

#[tokio::test]
async fn parse_artifact_repository_error_returns_failure() -> Result<(), Box<dyn std::error::Error>>
{
    struct FailingArtifactRepo;

    impl ArtifactRepository for FailingArtifactRepo {
        fn put(&self, _artifact: Artifact) -> Result<(), PortError> {
            Ok(())
        }

        fn get(&self, _id: ArtifactId) -> Result<Option<Artifact>, PortError> {
            Err(PortError::Internal {
                message: "simulated repo failure".into(),
            })
        }
    }

    let adapters = Adapters {
        artifact_repo: Arc::new(FailingArtifactRepo),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(99),
            source_path: "/repo/ghost.rs".to_string(),
            source_bytes: b"fn gone() {}".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(
        !result,
        "repository error should return false so retry policy remains active"
    );
    Ok(())
}
