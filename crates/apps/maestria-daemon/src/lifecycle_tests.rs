use super::{recovery_artifact_ids, validation_task_ids};
use crate::{InstanceLifecycle, RecoveryInputs, prepare_instance};
use maestria_domain::{
    ArtifactDetected, ArtifactId, DomainInput, ParserStarted, RequestTaskValidation,
    StartFullTextIndex, TaskId, content_hash,
};
use maestria_governance::AutonomyProfile;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    let input_tx = lifecycle.runtime_handle().feedback_sender();
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
