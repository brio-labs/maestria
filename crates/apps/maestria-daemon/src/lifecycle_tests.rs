use super::{
    RecoveryQueueStage, queue_recovery_inputs, recovery_artifact_ids, validation_task_ids,
};
use crate::{InstanceLifecycle, RecoveryInputs, prepare_instance};
use maestria_domain::{
    ArtifactDetected, ArtifactId, DomainInput, ParserStarted, RequestTaskValidation,
    StartFullTextIndex, TaskId, content_hash,
};
use maestria_governance::AutonomyProfile;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn create() -> std::io::Result<Self> {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "maestria-lifecycle-test-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn parser_input(id: u64) -> DomainInput {
    DomainInput::ResumeParser(ParserStarted {
        artifact_id: ArtifactId::new(id),
        title: format!("artifact-{id}"),
        source_path: format!("/tmp/artifact-{id}"),
        content_hash: format!("hash-{id}"),
        blob_id: maestria_domain::BlobId::new(id),
    })
}

#[tokio::test]
async fn interrupted_queue_preserves_remaining_inputs_for_ordered_retry()
-> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(1);
    let mut recovery = RecoveryInputs {
        resume_parsers: vec![parser_input(1), parser_input(2)],
        start_full_text: vec![DomainInput::StartFullTextIndex(StartFullTextIndex {
            artifact_id: ArtifactId::new(3),
        })],
        run_validations: vec![DomainInput::RequestTaskValidation(RequestTaskValidation {
            task_id: TaskId::new(4),
        })],
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        queue_recovery_inputs(
            &input_tx,
            &mut recovery.resume_parsers,
            RecoveryQueueStage::ResumeParser,
        ),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(recovery.resume_parsers, vec![parser_input(2)]);
    assert_eq!(input_rx.recv().await, Some(parser_input(1)));

    let receiver = tokio::spawn(async move {
        let mut received = Vec::new();
        while let Some(input) = input_rx.recv().await {
            received.push(input);
        }
        received
    });
    queue_recovery_inputs(
        &input_tx,
        &mut recovery.resume_parsers,
        RecoveryQueueStage::ResumeParser,
    )
    .await?;
    queue_recovery_inputs(
        &input_tx,
        &mut recovery.start_full_text,
        RecoveryQueueStage::FullText,
    )
    .await?;
    queue_recovery_inputs(
        &input_tx,
        &mut recovery.run_validations,
        RecoveryQueueStage::Validation,
    )
    .await?;
    drop(input_tx);

    let mut received = vec![parser_input(1)];
    received.extend(receiver.await?);
    assert_eq!(
        received,
        vec![
            parser_input(1),
            parser_input(2),
            DomainInput::StartFullTextIndex(StartFullTextIndex {
                artifact_id: ArtifactId::new(3),
            }),
            DomainInput::RequestTaskValidation(RequestTaskValidation {
                task_id: TaskId::new(4),
            }),
        ]
    );
    assert!(recovery.resume_parsers.is_empty());
    assert!(recovery.start_full_text.is_empty());
    assert!(recovery.run_validations.is_empty());
    Ok(())
}

#[tokio::test]
async fn closed_receiver_preserves_unsent_inputs_with_stage_context()
-> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, input_rx) = mpsc::channel(1);
    drop(input_rx);
    let mut inputs = vec![DomainInput::RequestTaskValidation(RequestTaskValidation {
        task_id: TaskId::new(9),
    })];

    let error = queue_recovery_inputs(&input_tx, &mut inputs, RecoveryQueueStage::Validation)
        .await
        .err();
    let message = error.map_or_else(String::new, |error| format!("{error:#}"));
    assert!(message.contains("task validation"));
    assert_eq!(inputs.len(), 1);
    Ok(())
}

#[test]
fn recovery_queue_ids_preserve_dependency_groups() {
    let recovery = RecoveryInputs {
        resume_parsers: vec![parser_input(1)],
        start_full_text: vec![DomainInput::StartFullTextIndex(StartFullTextIndex {
            artifact_id: ArtifactId::new(2),
        })],
        run_validations: vec![DomainInput::RequestTaskValidation(RequestTaskValidation {
            task_id: TaskId::new(3),
        })],
    };
    assert_eq!(
        recovery_artifact_ids(&recovery),
        vec![ArtifactId::new(1), ArtifactId::new(2)]
    );
    assert_eq!(validation_task_ids(&recovery), vec![TaskId::new(3)]);
}
#[tokio::test]
async fn internal_runtime_fatal_shutdown_does_not_require_external_signal()
-> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::create()?;
    let layout = prepare_instance(temp_dir.path().to_path_buf())?;
    let lifecycle = InstanceLifecycle::start(layout.clone(), AutonomyProfile::ReadOnly).await?;
    let external_shutdown = CancellationToken::new();
    let input_tx = lifecycle.input_sender();
    let source_bytes = b"# injected fatal effect\nAKIA1234567890123456".to_vec();
    input_tx
        .send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: ArtifactId::new(1),
            title: "fatal.md".to_string(),
            source_path: temp_dir.path().join("fatal.md").display().to_string(),
            content_hash: content_hash(&source_bytes),
            source_bytes,
        }))
        .await?;

    let lifecycle_result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        lifecycle.run_until_shutdown(external_shutdown.clone()),
    )
    .await?;
    let Some(error) = lifecycle_result.err() else {
        return Err("runtime fatal effect unexpectedly returned success".into());
    };

    assert!(
        !external_shutdown.is_cancelled(),
        "runtime failure must not cancel the caller's shutdown token"
    );
    assert!(
        layout.system_dir.join("watcher-state.json").exists(),
        "watcher state must be persisted during runtime-failure cleanup"
    );
    let message = format!("{error:#}");
    assert!(
        message.contains("runtime"),
        "runtime failure context must be preserved: {message}"
    );
    Ok(())
}
