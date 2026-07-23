use crate::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, ChunkId, DomainInput, EvidenceKind, IndexStatus, ParseArtifactRequest,
    ParseStatus,
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
        )?;
        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            artifact_version_id: maestria_domain::ArtifactVersionId::new(1),
            content_hash: maestria_domain::ContentHash::new(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            )
            .map_err(|e| maestria_ports::PortError::Internal {
                message: e.to_string(),
            })?,
            tree,
            status: maestria_ports::ParseStatus::Parsed,
            chunks,
            cards: vec![],
        })
    }
}

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

    fn parse(&self, _file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
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
        )?;
        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            artifact_version_id: maestria_domain::ArtifactVersionId::new(1),
            content_hash: maestria_domain::ContentHash::new(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            )
            .map_err(|e| maestria_ports::PortError::Internal {
                message: e.to_string(),
            })?,
            tree,
            status: maestria_ports::ParseStatus::Parsed,
            chunks: vec![chunk],
            cards: vec![],
        })
    }
}

async fn assert_pdf_span_evidence(
    input_rx: &mut mpsc::Receiver<DomainInput>,
    expected_pages: &[u32],
) -> Result<(), Box<dyn std::error::Error>> {
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
                    other => {
                        return Err(
                            format!("expected PdfSpan for evidence {i}, got {other:?}").into()
                        );
                    }
                }
            }
            Ok(Some(other)) => {
                return Err(format!("expected RecordEvidence {i}, got {other:?}").into());
            }
            Ok(None) => {
                return Err(format!("channel closed before RecordEvidence {i}").into());
            }
            Err(_) => {
                return Err(format!("timeout waiting for RecordEvidence {i}").into());
            }
        }
    }
    Ok(())
}
#[tokio::test]
async fn pdf_evidence_maps_page_one_to_pdf_span() -> Result<(), Box<dyn std::error::Error>> {
    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo.put(Artifact {
        id: ArtifactId::new(100),
        title: "pdf-doc".to_string(),
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
        .await?
        .ok_or("channel closed before ParserStarted")?;

    // Drain ParserCompleted.
    tokio::time::timeout(Duration::from_secs(1), input_rx.recv())
        .await?
        .ok_or("channel closed before ParserCompleted")?;

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
                other => return Err(format!("expected PdfSpan evidence, got {other:?}").into()),
            }
        }
        Ok(Some(other)) => return Err(format!("expected RecordEvidence, got {other:?}").into()),
        Ok(None) => return Err("channel closed before RecordEvidence".to_string().into()),
        Err(_) => return Err("timeout waiting for RecordEvidence".to_string().into()),
    }
    Ok(())
}

#[tokio::test]
async fn pdf_evidence_maps_page_n_to_pdf_span() -> Result<(), Box<dyn std::error::Error>> {
    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo.put(Artifact {
        id: ArtifactId::new(200),
        title: "multi-page-pdf".to_string(),
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
        .await?
        .ok_or("channel closed before ParserStarted")?;

    // Drain ParserCompleted.
    tokio::time::timeout(Duration::from_secs(1), input_rx.recv())
        .await?
        .ok_or("channel closed before ParserCompleted")?;

    // Collect evidence: pages 1, 3, 5 (empty pages 2 and 4 are skipped).
    let expected_pages = [1u32, 3, 5];
    assert_pdf_span_evidence(&mut input_rx, &expected_pages).await?;
    Ok(())
}

#[tokio::test]
async fn scanned_pdf_no_extractable_text_emits_terminal_parser_failure()
-> Result<(), Box<dyn std::error::Error>> {
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
    artifact_repo.put(Artifact {
        id: ArtifactId::new(300),
        title: "scanned".to_string(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::Unindexed,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    })?;

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
        result,
        "scanned/no-text PDF parser failure should be terminal"
    );

    // ParserStarted is sent before parsing as a crash-recovery marker.
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserStarted(ps))) => {
            assert_eq!(ps.artifact_id, ArtifactId::new(300));
        }
        Ok(Some(other)) => return Err(format!("expected ParserStarted, got {other:?}").into()),
        Ok(None) => return Err("channel closed before ParserStarted".to_string().into()),
        Err(_) => return Err("timeout waiting for ParserStarted".to_string().into()),
    }

    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserCompleted(pr))) => {
            assert_eq!(pr.status, ParseStatus::Failed);
            assert!(pr.chunks.is_empty());
        }
        Ok(Some(other)) => {
            return Err(format!("expected terminal ParserCompleted, got {other:?}").into());
        }
        Ok(None) => {
            return Err("channel closed before terminal parser result"
                .to_string()
                .into());
        }
        Err(_) => {
            return Err("timeout waiting for terminal parser result"
                .to_string()
                .into());
        }
    }
    Ok(())
}
