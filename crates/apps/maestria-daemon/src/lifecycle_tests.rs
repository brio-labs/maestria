use crate::recovery_staging::{recovery_artifact_ids, validation_task_ids};
use crate::test_support::TempDir;
use crate::{InstanceLifecycle, RecoveryInputs, prepare_instance};
use maestria_domain::{
    ArtifactDetected, ArtifactId, ContentHash, DomainInput, ParserStarted, RequestTaskValidation,
    StartFullTextIndex, TaskId, content_hash,
};
use maestria_governance::AutonomyProfile;
use tokio_util::sync::CancellationToken;

fn parser_input(id: u64) -> Result<DomainInput, Box<dyn std::error::Error>> {
    Ok(DomainInput::ResumeParser(ParserStarted {
        artifact_id: ArtifactId::new(id),
        title: format!("artifact-{id}"),
        source_path: format!("/tmp/artifact-{id}"),
        content_hash: ContentHash::new(format!("sha256:{:064x}", id))?,
        blob_id: maestria_domain::BlobId::new(id),
    }))
}

#[test]
fn recovery_queue_ids_preserve_dependency_groups() -> Result<(), Box<dyn std::error::Error>> {
    let recovery = RecoveryInputs {
        resume_parsers: vec![parser_input(1)?],
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
    Ok(())
}
#[tokio::test]
async fn secret_bearing_artifact_is_quarantined_and_runtime_continues()
-> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::create()?;
    let layout = prepare_instance(temp_dir.path().to_path_buf())?;
    let lifecycle = InstanceLifecycle::start(layout.clone(), AutonomyProfile::ReadOnly).await?;
    let external_shutdown = CancellationToken::new();
    let input_tx = lifecycle.runtime_handle().feedback_sender();
    let source_bytes = b"# injected secret\nAKIA1234567890123456".to_vec();
    input_tx
        .send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: ArtifactId::new(1),
            title: "secret.md".to_string(),
            source_path: temp_dir.path().join("secret.md").display().to_string(),
            content_hash: ContentHash::new(content_hash(&source_bytes))?,
            source_bytes,
        }))
        .await?;
    let mut run = tokio::spawn(lifecycle.run_until_shutdown(external_shutdown.clone()));

    // Secret-bearing content is a per-artifact privacy outcome: the artifact
    // must reach the quarantined terminal state instead of killing the
    // runtime (per-artifact quarantine / failure isolation).
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let store =
                maestria_storage_sqlite::SqliteStore::open_read_only(&layout.database_path)?;
            let artifact = maestria_ports::ArtifactRepository::get(&store, ArtifactId::new(1))?;
            if artifact.and_then(|artifact| artifact.parse_status)
                == Some(maestria_domain::ParseStatus::Quarantined)
            {
                return Ok::<(), Box<dyn std::error::Error>>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| "artifact never reached the quarantined terminal state")??;

    // The runtime must still be serving: the lifecycle must not have
    // terminated on its own after the quarantine.
    let still_running = tokio::time::timeout(std::time::Duration::from_secs(1), &mut run)
        .await
        .is_err();
    assert!(
        still_running,
        "runtime must continue serving after quarantining a secret-bearing artifact"
    );
    assert!(
        !external_shutdown.is_cancelled(),
        "quarantine must not cancel the caller's shutdown token"
    );

    external_shutdown.cancel();
    run.await
        .map_err(|join| join.to_string())?
        .map_err(|error| format!("clean external shutdown failed: {error:#}"))?;
    Ok(())
}
