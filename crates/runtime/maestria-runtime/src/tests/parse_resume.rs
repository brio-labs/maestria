use crate::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, ContentHash, IndexStatus, ParseArtifactRequest, ParserStarted,
};
use maestria_ports::{ArtifactRepository, BlobStore, InMemoryArtifactRepository};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

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

    let expected_content_hash = ContentHash::new(content_hash(expected_bytes))?;
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
