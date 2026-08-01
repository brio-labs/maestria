use crate::test_support::*;
use maestria_domain::{Artifact, ArtifactId, DomainInput, IndexStatus, ParseArtifactRequest};
use maestria_ports::{
    ArtifactRepository, BlobStore, FileHandle, FileMetadata, InMemoryArtifactRepository,
    ParseContext, ParsedArtifact, Parser, PortError,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

struct NeedsOcrTestParser;

impl Parser for NeedsOcrTestParser {
    fn id(&self) -> &'static str {
        "needs-ocr-test"
    }

    fn supports(&self, _file: &FileMetadata) -> bool {
        true
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        maestria_ports::InMemoryParser::new().parse(file, context)
    }

    fn parse_outcome(
        &self,
        file: FileHandle,
        context: ParseContext,
    ) -> Result<maestria_ports::ParseOutcome, PortError> {
        let parsed = self.parse(file, context)?;
        Ok(maestria_ports::ParseOutcome::NeedsOcr {
            partial: ParsedArtifact {
                status: maestria_ports::ParseStatus::NeedsOcr,
                ..parsed
            },
            pages: maestria_ports::OcrPageSet::try_new([1])?,
        })
    }

    fn parse_with_ocr(
        &self,
        file: FileHandle,
        context: ParseContext,
        pages: &[maestria_domain::OcrPageText],
    ) -> Result<ParsedArtifact, PortError> {
        assert!(!pages.is_empty());
        self.parse(file, context)
    }
}

#[tokio::test]
async fn needs_ocr_without_provider_emits_empty_pending_completion()
-> Result<(), Box<dyn std::error::Error>> {
    let adapters = Adapters {
        parser: Arc::new(NeedsOcrTestParser),
        ..crate::test_helpers::test_adapters()
    };
    let (input_tx, mut input_rx) = mpsc::channel(8);
    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(crate::test_helpers::test_governance()),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );

    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(801),
            source_path: "/repo/scanned.pdf".into(),
            source_bytes: b"scanned".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;
    assert!(result);

    assert!(matches!(
        input_rx.recv().await,
        Some(DomainInput::ParserStarted(_))
    ));
    let Some(DomainInput::ParserCompleted(completion)) = input_rx.recv().await else {
        return Err("expected pending ParserCompleted".into());
    };
    assert_eq!(completion.status, maestria_domain::ParseStatus::NeedsOcr);
    assert!(completion.chunks.is_empty());
    assert!(completion.cards.is_empty());
    match tokio::time::timeout(Duration::from_millis(50), input_rx.recv()).await {
        Err(_) | Ok(None) => {}
        Ok(Some(unexpected)) => {
            return Err(format!("unexpected input before OCR completion: {unexpected:?}").into());
        }
    }
    Ok(())
}

fn ocr_intent_for_source(
    artifact_id: ArtifactId,
    blob_id: maestria_domain::BlobId,
    source_hash: &str,
    provider: &str,
) -> Result<maestria_domain::OcrIntent, Box<dyn std::error::Error>> {
    Ok(maestria_domain::OcrIntent::new(
        artifact_id,
        blob_id,
        maestria_domain::ContentHash::new(source_hash.to_string())?,
        [1],
        maestria_domain::OcrProviderIdentity::new(
            provider,
            "model",
            "revision",
            "sha256:provider",
            "prep",
        )?,
        maestria_domain::OcrDisclosure::new(
            false,
            maestria_domain::OcrRetentionPolicy::NoRetention,
        ),
    )?)
}

#[tokio::test]
async fn completed_ocr_is_selected_even_when_older_intent_is_pending()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = b"recoverable scanned text";
    let artifact_id = ArtifactId::new(802);
    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo.put(Artifact {
        id: artifact_id,
        title: "scanned".into(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::Unindexed,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    })?;
    let blob_store = Arc::new(InMemoryBlobStore::new());
    let blob_id = blob_store.put(bytes.to_vec())?;
    let hash = maestria_domain::content_hash(bytes);
    let mut intents = vec![
        ocr_intent_for_source(artifact_id, blob_id, &hash, "first")?,
        ocr_intent_for_source(artifact_id, blob_id, &hash, "second")?,
    ];
    intents.sort_by(|left, right| left.request_id().cmp(right.request_id()));
    let pending_intent = intents.remove(0);
    let completed_intent = intents.remove(0);
    let completion = maestria_domain::OcrCompletion::new(
        &completed_intent,
        [maestria_domain::OcrPageText::new(1, "recognized")?],
    )?;
    let mut state = KernelState::new();
    state.pending_parsers.insert(
        artifact_id,
        maestria_domain::ParserStarted {
            artifact_id,
            title: "scanned".into(),
            source_path: "/repo/scanned.pdf".into(),
            content_hash: hash,
            blob_id,
        },
    );
    state
        .ocr_intents
        .insert(pending_intent.request_id().clone(), pending_intent.clone());
    state
        .pending_ocr
        .insert(pending_intent.request_id().clone(), pending_intent);
    state.ocr_intents.insert(
        completed_intent.request_id().clone(),
        completed_intent.clone(),
    );
    state
        .ocr_results
        .insert(completed_intent.request_id().clone(), completion);

    let adapters = Adapters {
        parser: Arc::new(NeedsOcrTestParser),
        blob_store,
        artifact_repo: Arc::new(artifact_repo),
        ..crate::test_helpers::test_adapters()
    };
    let (input_tx, mut input_rx) = mpsc::channel(8);
    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(crate::test_helpers::test_governance()),
        Arc::new(RwLock::new(state)),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id,
            source_path: "/repo/scanned.pdf".into(),
            source_bytes: Vec::new(),
            source_blob: Some(blob_id),
        }),
        ctx,
        None,
    )
    .await;
    assert!(result);

    let Some(DomainInput::ParserCompleted(completion)) = input_rx.recv().await else {
        return Err("expected completion from durable OCR".into());
    };
    assert_eq!(completion.status, maestria_domain::ParseStatus::Parsed);
    assert!(!completion.chunks.is_empty());
    loop {
        match tokio::time::timeout(Duration::from_millis(50), input_rx.recv()).await {
            Err(_) | Ok(None) => break,
            Ok(Some(DomainInput::OcrRequested(_))) => {
                return Err("durable OCR completion was retransmitted".into());
            }
            Ok(Some(_)) => {}
        }
    }
    Ok(())
}
