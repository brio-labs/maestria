use super::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, BlobId, Evidence, EvidenceKind, IndexStatus, LogicalTick,
    ParseArtifactRequest,
};
use maestria_ports::{ArtifactRepository, BlobStore, InMemoryArtifactRepository, PortError};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

/// BlobStore that records every `put` payload for later assertion.
struct RecordingBlobStore {
    recorded: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
}

impl BlobStore for RecordingBlobStore {
    fn put(&self, bytes: Vec<u8>) -> Result<BlobId, PortError> {
        match self.recorded.lock() {
            Ok(mut guard) => {
                let id = guard.len() as u64 + 1;
                guard.push(bytes);
                Ok(BlobId::new(id))
            }
            Err(_poisoned) => Err(PortError::Internal {
                message: "recording blob store lock poisoned".to_string(),
            }),
        }
    }

    fn get(&self, _id: BlobId) -> Result<Vec<u8>, PortError> {
        Err(PortError::NotFound)
    }
}

#[tokio::test]
async fn parse_artifact_calls_blob_put_exactly_once() -> Result<(), Box<dyn std::error::Error>> {
    let recorded: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let blob_store = Arc::new(RecordingBlobStore {
        recorded: recorded.clone(),
    });

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo.put(Artifact {
        id: ArtifactId::new(55),
        title: "single-put".into(),
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
        blob_store,
        artifact_repo: Arc::new(artifact_repo),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let source_bytes = b"single put test content".to_vec();
    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(55),
            source_path: "/repo/single.rs".to_string(),
            source_bytes: source_bytes.clone(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(result, "ParseArtifact should succeed");
    match recorded.lock() {
        Ok(guard) => {
            assert_eq!(guard.len(), 1, "exactly one blob put per ParseArtifact");
            assert_eq!(guard[0], source_bytes, "payload matches");
        }
        Err(poisoned) => return Err(format!("recording mutex poisoned: {:?}", poisoned).into()),
    }
    Ok(())
}

#[tokio::test]
async fn parse_artifact_retry_redrives_existing_evidence() -> Result<(), Box<dyn std::error::Error>>
{
    let artifact_id = ArtifactId::new(77);
    let existing_evidence_id = evidence_id_for(artifact_id, 0);

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo.put(Artifact {
        id: ArtifactId::new(77),
        title: "retry-test".into(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::from([existing_evidence_id]),
        index_status: IndexStatus::Unindexed,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    })?;

    // Pre-populate state with existing evidence to simulate replay/retry.
    let mut state = KernelState::new();
    state.evidences.insert(
        existing_evidence_id,
        Evidence {
            id: existing_evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/repo/retry.rs".into(),
                range: maestria_domain::ContentRange { start: 1, end: 1 },
                content_hash: content_hash(b"existing"),
                snapshot: Some(BlobId::new(1)),
            },
            excerpt: "existing excerpt".into(),
            observed_at: LogicalTick::new(1),
            security: maestria_domain::SecurityMetadata::default(),
        },
    );

    let adapters = Adapters {
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
            source_path: "/repo/retry.rs".to_string(),
            source_bytes: b"retry content".to_vec(),
            source_blob: None,
        }),
        ctx,
        None,
    )
    .await;

    assert!(result, "ParseArtifact should succeed even on retry");

    // First input after retry: ParserStarted (fresh ingestion sends it before parsing).
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserStarted(_ps))) => {
            // ParserStarted acknowledged.
        }
        Ok(Some(DomainInput::RecordEvidence(ev))) => {
            return Err(format!(
                "unexpected RecordEvidence before ParserStarted for evidence_id {ev:?}"
            )
            .into());
        }
        Ok(Some(other)) => return Err(format!("expected ParserStarted, got {other:?}").into()),
        Ok(None) => return Err("channel closed before ParserStarted".to_string().into()),
        Err(_) => return Err("timeout waiting for ParserStarted".to_string().into()),
    }

    // Second input: ParserCompleted.
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserCompleted(pr))) => {
            assert_eq!(pr.artifact_id, artifact_id);
            assert_eq!(pr.chunks.len(), 1);
        }
        Ok(Some(other)) => return Err(format!("expected ParserCompleted, got {other:?}").into()),
        Ok(None) => return Err("channel closed before ParserCompleted".to_string().into()),
        Err(_) => return Err("timeout waiting for ParserCompleted".to_string().into()),
    }

    // Existing evidence is re-driven so malformed persisted evidence can be
    // repaired; valid duplicates are idempotent in the domain reducer.
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::RecordEvidence(ev))) => {
            assert_eq!(ev.evidence_id, existing_evidence_id);
            assert_eq!(ev.artifact_id, artifact_id);
        }
        Ok(Some(other)) => return Err(format!("expected RecordEvidence, got {other:?}").into()),
        Ok(None) => return Err("channel closed before RecordEvidence".to_string().into()),
        Err(_) => return Err("timeout waiting for RecordEvidence".to_string().into()),
    }
    Ok(())
}
