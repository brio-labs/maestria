use crate::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, DomainInput, EvidenceKind, IndexStatus, LogicalTick,
    ParseArtifactRequest, ParserStarted,
};
use maestria_ports::{
    ArtifactRepository, FileHandle, FileMetadata, InMemoryArtifactRepository, ParseContext,
    ParsedArtifact, ParsedChunk, Parser, PortError, SourceSpan,
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
async fn parse_artifact_empty_bytes_emit_terminal_failure() -> Result<(), Box<dyn std::error::Error>>
{
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

    assert!(
        result,
        "invalid parser input should be terminal, not retried"
    );
    Ok(())
}
#[tokio::test]
async fn parse_artifact_unsupported_parser_emits_terminal_status()
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
        result,
        "unsupported parser should be terminal and not retried"
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

struct MismatchedArtifactIdParser;

impl Parser for MismatchedArtifactIdParser {
    fn id(&self) -> &'static str {
        "mismatched-artifact-id"
    }

    fn supports(&self, _file: &FileMetadata) -> bool {
        true
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let mismatched_artifact_id = ArtifactId::new(context.artifact_id.value() + 1);
        let parsed = maestria_ports::InMemoryParser::new().parse(file, context)?;
        Ok(ParsedArtifact {
            artifact_id: mismatched_artifact_id,
            ..parsed
        })
    }
}
struct MismatchedHashParser;

impl Parser for MismatchedHashParser {
    fn id(&self) -> &'static str {
        "mismatched-hash"
    }

    fn supports(&self, _file: &FileMetadata) -> bool {
        true
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let actual_hash = maestria_domain::content_hash(&file.bytes);
        let wrong_hash = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        if actual_hash == wrong_hash {
            return Err(PortError::Internal {
                message: "test hash must differ from source hash".to_string(),
            });
        }
        let node_id = maestria_domain::StructureNodeId::new(context.artifact_id.value());
        let tree = maestria_ports::DocumentTree::new(
            node_id,
            vec![maestria_domain::StructureNode {
                id: node_id,
                parent_id: None,
                sibling_id: None,
                node_type: maestria_domain::StructureNodeType::Document,
                source_range: maestria_domain::ContentRange { start: 0, end: 1 },
                page: None,
                section_path: vec![],
                parser_generation: "test".to_string(),
                schema_generation: "v1".to_string(),
                language: None,
            }],
        )
        .map_err(|e| PortError::Internal {
            message: format!("{e:?}"),
        })?;
        let chunk = ParsedChunk {
            chunk_id: maestria_domain::ChunkId::new(context.artifact_id.value()),
            artifact_id: context.artifact_id,
            node_id,
            text: "irrelevant".to_string(),
            representations: vec![],
            source_span: SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
        };
        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            artifact_version_id: maestria_domain::ArtifactVersionId::new(1),
            content_hash: maestria_domain::ContentHash::new(wrong_hash.to_string()).map_err(
                |e| PortError::Internal {
                    message: e.to_string(),
                },
            )?,
            tree,
            status: maestria_ports::ParseStatus::Parsed,
            chunks: vec![chunk],
            cards: vec![],
        })
    }
}

#[tokio::test]
async fn parse_artifact_mismatched_content_hash_rejects_completion()
-> Result<(), Box<dyn std::error::Error>> {
    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo.put(Artifact {
        id: ArtifactId::new(77),
        title: "mismatch-test".to_string(),
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
        parser: Arc::new(MismatchedHashParser),
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
            artifact_id: ArtifactId::new(77),
            source_path: "/repo/mismatch.rs".to_string(),
            source_bytes: b"fn mismatch() {}".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    if result {
        return Err("mismatched content hash should reject parse".into());
    }
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserStarted(ps))) => {
            if ps.artifact_id != ArtifactId::new(77) {
                return Err("ParserStarted carried the wrong artifact".into());
            }
        }
        other => return Err(format!("expected ParserStarted, got {other:?}").into()),
    }
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(None) | Err(_) => Ok(()),
        Ok(Some(unexpected)) => Err(format!(
            "expected no further inputs after hash mismatch, got {unexpected:?}"
        )
        .into()),
    }
}

#[tokio::test]
async fn parse_artifact_mismatched_artifact_id_rejects_completion_and_index_inputs()
-> Result<(), Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(78);
    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo.put(Artifact {
        id: artifact_id,
        title: "identity-mismatch-test".to_string(),
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
        parser: Arc::new(MismatchedArtifactIdParser),
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
            artifact_id,
            source_path: "/repo/identity.rs".to_string(),
            source_bytes: b"fn identity() {}".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(!result, "cross-artifact parser output must be rejected");
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserStarted(started))) => {
            assert_eq!(started.artifact_id, artifact_id);
        }
        other => return Err(format!("expected ParserStarted, got {other:?}").into()),
    }
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(unexpected)) => {
            Err(format!("cross-artifact parse emitted an unexpected input: {unexpected:?}").into())
        }
        Ok(None) | Err(_) => Ok(()),
    }
}

#[tokio::test]
async fn resume_parse_rejects_blob_when_durable_parser_hash_differs()
-> Result<(), Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(79);
    let expected_bytes = b"durable resume bytes";
    let blob_store = Arc::new(InMemoryBlobStore::new());
    let blob_id = blob_store.put(b"tampered resume bytes".to_vec())?;

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo.put(Artifact {
        id: artifact_id,
        title: "resume-hash-test".to_string(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::Unindexed,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    })?;

    let expected_content_hash = content_hash(expected_bytes);
    let mut state = KernelState::new();
    state.pending_parsers.insert(
        artifact_id,
        ParserStarted {
            artifact_id,
            title: "resume-hash-test".to_string(),
            source_path: "/repo/resume-hash.rs".to_string(),
            content_hash: expected_content_hash,
            blob_id,
        },
    );

    let adapters = Adapters {
        blob_store,
        artifact_repo: Arc::new(artifact_repo),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (input_tx, mut input_rx) = mpsc::channel(8);
    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(state)),
        input_tx,
    );

    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id,
            source_path: "/repo/resume-hash.rs".to_string(),
            source_bytes: Vec::new(),
            source_blob: Some(blob_id),
        }),
        ctx,
        None,
    )
    .await;

    assert!(!result, "resume with tampered bytes must be rejected");
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(unexpected)) => {
            Err(format!("wrong resume blob emitted an unexpected input: {unexpected:?}").into())
        }
        Ok(None) | Err(_) => Ok(()),
    }
}
