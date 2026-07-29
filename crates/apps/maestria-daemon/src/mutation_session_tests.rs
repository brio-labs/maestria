use super::MutationSession;
use crate::{load_kernel_state, prepare_instance};
use maestria_domain::{
    DomainInput, EvidenceId, LinkEvidenceToTaskInput, OpenTaskInput, TaskId, TaskPriority,
};
use maestria_governance::AutonomyProfile;
use maestria_runtime::RuntimeSubmissionError;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn create() -> std::io::Result<Self> {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "maestria-mutation-session-test-{}-{id}",
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

#[tokio::test]
async fn correlated_mutation_is_durable_before_graceful_finish()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::create()?;
    let layout = prepare_instance(temp.path().to_path_buf())?;
    let session = MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace).await?;
    let task_id = TaskId::new(1);

    let application = session
        .submit(DomainInput::OpenTask(OpenTaskInput {
            task_id,
            title: "lifecycle-owned mutation".to_string(),
            priority: TaskPriority::Low,
            artifact_id: None,
        }))
        .await?;
    assert_eq!(application.events.len(), 1);
    session.finish(Ok(())).await?;

    let state = load_kernel_state(&layout)?;
    assert!(state.tasks.contains_key(&task_id));
    Ok(())
}

#[tokio::test]
async fn typed_rejection_survives_finish_and_releases_instance_lock()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::create()?;
    let layout = prepare_instance(temp.path().to_path_buf())?;
    let session = MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace).await?;

    let submission = session
        .submit(DomainInput::LinkEvidenceToTask(LinkEvidenceToTaskInput {
            task_id: TaskId::new(99),
            evidence_id: EvidenceId::new(88),
        }))
        .await;
    assert!(matches!(
        submission,
        Err(RuntimeSubmissionError::DomainRejected { .. })
    ));
    let operation = submission.map(|_| ()).map_err(anyhow::Error::from);
    let finished = session.finish(operation).await;
    let Some(error) = finished.err() else {
        return Err("rejected mutation unexpectedly finished successfully".into());
    };
    assert!(error.to_string().contains("domain rejected input"));

    let restarted = MutationSession::start(layout, AutonomyProfile::TrustedWorkspace).await?;
    restarted.finish(Ok(())).await?;
    Ok(())
}
