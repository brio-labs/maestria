#![allow(clippy::disallowed_methods)]
#![allow(clippy::too_many_lines)]

use super::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, ChunkId, EvidenceKind, IndexStatus, ParseArtifactRequest,
};
use maestria_ports::{
    ArtifactRepository, FileHandle, FileMetadata, InMemoryArtifactRepository, InMemoryEventLog,
    ParseContext, ParsedArtifact, ParsedChunk, Parser, PortError, SourceSpan,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

struct PageFivePdfParser;
impl Parser for PageFivePdfParser {
    fn id(&self) -> &'static str {
        "page-five-pdf"
    }
    fn supports(&self, file: &FileMetadata) -> bool {
        file.extension
            .as_deref()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
    }
    fn parse(&self, _file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let chunks: Vec<ParsedChunk> = [1, 3, 5]
            .into_iter()
            .enumerate()
            .map(|(order, page)| ParsedChunk {
                chunk_id: ChunkId::new(context.artifact_id.value().wrapping_add(order as u64)),
                artifact_id: context.artifact_id,
                node_id: maestria_domain::StructureNodeId::new(order as u64),
                text: format!("page {page} content"),
                representations: vec![],
                source_span: SourceSpan::PdfSpan { page },
            })
            .collect();
        let tree = maestria_ports::DocumentTree::new(
            maestria_domain::StructureNodeId::new(0),
            vec![maestria_domain::StructureNode {
                id: maestria_domain::StructureNodeId::new(0),
                parent_id: None,
                sibling_id: None,
                node_type: maestria_domain::StructureNodeType::Document,
                source_range: maestria_domain::ContentRange { start: 0, end: 100 },
                page: None,
                section_path: vec![],
                parser_generation: "1".to_string(),
                schema_generation: "1".to_string(),
                language: None,
            }],
        )
        .unwrap();
        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            artifact_version_id: maestria_domain::ArtifactVersionId::new(1),
            content_hash: maestria_domain::ContentHash::new(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            )
            .unwrap(),
            tree,
            status: maestria_ports::ParseStatus::Parsed,
            chunks,
            cards: vec![],
        })
    }
}

async fn assert_pdf_span_evidence(
    input_rx: &mut mpsc::Receiver<DomainInput>,
    expected_pages: &[u32],
) {
    for (i, expected_page) in expected_pages.iter().enumerate() {
        match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
            Ok(Some(DomainInput::RecordEvidence(ev))) => {
                assert_eq!(ev.artifact_id, ArtifactId::new(200));
                match &ev.kind {
                    EvidenceKind::PdfSpan {
                        blob,
                        page_start,
                        page_end,
                    } => {
                        assert!(blob.value() > 0, "blob id must be non-zero");
                        assert_eq!(
                            *page_start, *expected_page,
                            "page_start mismatch for evidence {i}"
                        );
                        assert_eq!(
                            *page_end, *expected_page,
                            "page_end must match page_start for evidence {i}"
                        );
                    }
                    other => panic!("expected PdfSpan for evidence {i}, got {other:?}"),
                }
            }
            Ok(Some(other)) => panic!("expected RecordEvidence {i}, got {other:?}"),
            Ok(None) => panic!("channel closed before RecordEvidence {i}"),
            Err(_) => panic!("timeout waiting for RecordEvidence {i}"),
        }
    }
}
#[tokio::test]
async fn pdf_evidence_maps_page_one_to_pdf_span() {
    struct PageOnePdfParser;
    impl Parser for PageOnePdfParser {
        fn id(&self) -> &'static str {
            "page-one-pdf"
        }
        fn supports(&self, file: &FileMetadata) -> bool {
            file.extension
                .as_deref()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
        }
        fn parse(
            &self,
            _file: FileHandle,
            context: ParseContext,
        ) -> Result<ParsedArtifact, PortError> {
            let chunk = ParsedChunk {
                chunk_id: ChunkId::new(context.artifact_id.value()),
                artifact_id: context.artifact_id,
                node_id: maestria_domain::StructureNodeId::new(1),
                text: "page one content".to_string(),
                representations: vec![],
                source_span: SourceSpan::PdfSpan { page: 1 },
            };
            let tree = maestria_ports::DocumentTree::new(
                maestria_domain::StructureNodeId::new(0),
                vec![maestria_domain::StructureNode {
                    id: maestria_domain::StructureNodeId::new(0),
                    parent_id: None,
                    sibling_id: None,
                    node_type: maestria_domain::StructureNodeType::Document,
                    source_range: maestria_domain::ContentRange { start: 0, end: 100 },
                    page: None,
                    section_path: vec![],
                    parser_generation: "1".to_string(),
                    schema_generation: "1".to_string(),
                    language: None,
                }],
            )
            .unwrap();
            Ok(ParsedArtifact {
                artifact_id: context.artifact_id,
                artifact_version_id: maestria_domain::ArtifactVersionId::new(1),
                content_hash: maestria_domain::ContentHash::new(
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                )
                .unwrap(),
                tree,
                status: maestria_ports::ParseStatus::Parsed,
                chunks: vec![chunk],
                cards: vec![],
            })
        }
    }

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: ArtifactId::new(100),
            title: "pdf-doc".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
        })
        .expect("pre-populated artifact should be accepted");

    let adapters = Adapters {
        parser: Arc::new(PageOnePdfParser),
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
            artifact_id: ArtifactId::new(100),
            source_path: "/repo/doc.pdf".to_string(),
            source_bytes: b"%PDF-1.4 fake".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(result, "PDF parse with page 1 should succeed");

    // Drain ParserStarted.
    tokio::time::timeout(Duration::from_secs(1), input_rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed before ParserStarted");

    // Drain ParserCompleted.
    tokio::time::timeout(Duration::from_secs(1), input_rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed before ParserCompleted");

    // Third: RecordEvidence must carry PdfSpan with page_start=1, page_end=1.
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::RecordEvidence(ev))) => {
            assert_eq!(ev.artifact_id, ArtifactId::new(100));
            match &ev.kind {
                EvidenceKind::PdfSpan {
                    blob,
                    page_start,
                    page_end,
                } => {
                    assert!(blob.value() > 0, "blob id must be non-zero");
                    assert_eq!(*page_start, 1, "page_start must be 1");
                    assert_eq!(*page_end, 1, "page_end must equal page_start");
                }
                other => panic!("expected PdfSpan evidence, got {other:?}"),
            }
        }
        Ok(Some(other)) => panic!("expected RecordEvidence, got {other:?}"),
        Ok(None) => panic!("channel closed before RecordEvidence"),
        Err(_) => panic!("timeout waiting for RecordEvidence"),
    }
}

#[tokio::test]
async fn pdf_evidence_maps_page_n_to_pdf_span() {
    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: ArtifactId::new(200),
            title: "multi-page-pdf".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
        })
        .expect("pre-populated artifact should be accepted");

    let adapters = Adapters {
        parser: Arc::new(PageFivePdfParser),
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
            artifact_id: ArtifactId::new(200),
            source_path: "/repo/multi.pdf".to_string(),
            source_bytes: b"%PDF-1.7 multi-page".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(result, "multi-page PDF parse should succeed");

    // Drain ParserStarted.
    tokio::time::timeout(Duration::from_secs(1), input_rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed before ParserStarted");

    // Drain ParserCompleted.
    tokio::time::timeout(Duration::from_secs(1), input_rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed before ParserCompleted");

    // Collect evidence: pages 1, 3, 5 (empty pages 2 and 4 are skipped).
    let expected_pages = [1u32, 3, 5];
    assert_pdf_span_evidence(&mut input_rx, &expected_pages).await;
}

#[tokio::test]
async fn scanned_pdf_no_extractable_text_fails_before_parser_completed() {
    struct ScannedPdfParser;
    impl Parser for ScannedPdfParser {
        fn id(&self) -> &'static str {
            "scanned-pdf"
        }
        fn supports(&self, file: &FileMetadata) -> bool {
            file.extension
                .as_deref()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
        }
        fn parse(
            &self,
            _file: FileHandle,
            _context: ParseContext,
        ) -> Result<ParsedArtifact, PortError> {
            Err(PortError::InvalidInput {
                message: "PDF has no extractable text".to_string(),
            })
        }
    }

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: ArtifactId::new(300),
            title: "scanned".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
        })
        .expect("pre-populated artifact should be accepted");

    let event_log = Arc::new(InMemoryEventLog::new());
    let adapters = Adapters {
        event_log: event_log.clone(),
        parser: Arc::new(ScannedPdfParser),
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
            artifact_id: ArtifactId::new(300),
            source_path: "/repo/scanned.pdf".to_string(),
            source_bytes: b"%PDF-1.4 scanned image only".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(
        !result,
        "scanned/no-text PDF must fail before ParserCompleted"
    );

    // ParserStarted is sent before parsing as a crash-recovery marker.
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserStarted(ps))) => {
            assert_eq!(ps.artifact_id, ArtifactId::new(300));
        }
        Ok(Some(other)) => panic!("expected ParserStarted, got {other:?}"),
        Ok(None) => panic!("channel closed before ParserStarted"),
        Err(_) => panic!("timeout waiting for ParserStarted"),
    }

    // No further events must be emitted after parser failure.
    match tokio::time::timeout(Duration::from_millis(200), input_rx.recv()).await {
        Ok(Some(msg)) => panic!("unexpected event after parser failure: {msg:?}"),
        Ok(None) => {} // channel closed — acceptable
        Err(_) => {}   // timeout — no events, expected
    }
}
