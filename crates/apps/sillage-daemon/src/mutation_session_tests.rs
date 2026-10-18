use super::MutationSession;
use crate::test_support::TempDir;
use crate::{load_kernel_state, prepare_instance};
use sillage_core::{InstanceManifest, build_artifact_detected_input};
use sillage_domain::{
    DomainInput, EvidenceId, LinkEvidenceToTaskInput, OpenTaskInput, TaskId, TaskPriority,
};
use sillage_governance::AutonomyProfile;
use sillage_governance::RetrievalSecurityPolicy;
use sillage_ports::ArtifactRepository;
use sillage_runtime::RuntimeSubmissionError;
use sillage_storage_sqlite::SqliteStore;
use std::{collections::BTreeSet, fs};

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
async fn try_start_for_search_degrades_while_instance_lock_is_held()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::create()?;
    let layout = prepare_instance(temp.path().to_path_buf())?;
    let session = MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace).await?;

    let degraded =
        MutationSession::try_start_for_search(layout.clone(), AutonomyProfile::TrustedWorkspace)
            .await?;
    assert!(
        degraded.is_none(),
        "expected read-only degradation while the instance lock is held"
    );

    session.finish(Ok(())).await?;
    let durable =
        MutationSession::try_start_for_search(layout, AutonomyProfile::TrustedWorkspace).await?;
    let Some(durable) = durable else {
        return Err("expected a durable search session after the lock is released".into());
    };
    durable.finish(Ok(())).await?;
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

#[tokio::test]
async fn finish_settles_six_accepted_markdown_sources_before_returning()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::create()?;
    let source_root = temp.path().join("sources");
    fs::create_dir(&source_root)?;
    let layout = crate::prepare_instance_with_roots(
        temp.path().join("instance"),
        vec![source_root.clone()],
    )?;
    let session = MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace).await?;
    let cases = [
        ("amber.md", "amber sextant"),
        ("cobalt.md", "cobalt compass"),
        ("indigo.md", "indigo lantern"),
        ("ochre.md", "ochre telescope"),
        ("silver.md", "silver astrolabe"),
        ("violet.md", "violet chronometer"),
    ];
    let mut sources = Vec::with_capacity(cases.len());
    for (filename, query) in cases {
        let source_path = source_root.join(filename);
        let bytes = format!("# Field notes\n\nThe {query} guides this route.\n").into_bytes();
        fs::write(&source_path, &bytes)?;
        let input = build_artifact_detected_input(
            &source_path,
            bytes.clone(),
            sillage_domain::content_hash(&bytes),
        )?;
        let artifact_id = match &input {
            DomainInput::ArtifactDetected(source) => source.artifact_id,
            _ => return Err("canonical source builder returned another input kind".into()),
        };
        session.submit(input).await?;
        sources.push((artifact_id, query));
    }

    session.finish(Ok(())).await?;

    let state = load_kernel_state(&layout)?;
    let manifest = InstanceManifest::decode(&fs::read_to_string(&layout.manifest_path)?)?;
    let runtime = crate::prepare_search_runtime(
        &layout,
        &state,
        &manifest,
        RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
    )?;
    let store = SqliteStore::open(&layout.database_path)?;
    let expected_artifacts = sources
        .iter()
        .map(|(artifact_id, _)| *artifact_id)
        .collect::<BTreeSet<_>>();
    let mut retrieved_artifacts = BTreeSet::new();
    let mut retrieved_citations = BTreeSet::new();
    for (artifact_id, query) in &sources {
        let artifact = store
            .get(*artifact_id)?
            .ok_or("finished source is absent from durable storage")?;
        assert!(
            artifact.index_status == sillage_domain::IndexStatus::Indexed,
            "finish must settle every accepted parser/index pipeline"
        );
        let (_, outcome) = runtime.execute((*query).to_owned(), 10).await?;
        let citation = outcome
            .evidence
            .iter()
            .map(|candidate| candidate.evidence_id())
            .find(|evidence_id| artifact.evidence_ids.contains(evidence_id))
            .ok_or("retrieval returned no source citation")?;
        retrieved_artifacts.insert(*artifact_id);
        retrieved_citations.insert(citation);
    }
    assert!(
        retrieved_artifacts == expected_artifacts && retrieved_artifacts.len() == 6,
        "finish must expose exactly the six submitted source identities"
    );
    assert!(
        retrieved_citations.len() == 6,
        "finish must expose one distinct current citation for every source"
    );
    Ok(())
}

#[tokio::test]
async fn unresolved_submission_permit_fails_finish_without_advancing_watermark()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::create()?;
    let layout = prepare_instance(temp.path().to_path_buf())?;
    let session = MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace).await?;
    let permit = session.runtime_handle().reserve_submission().await?;
    let previous_watermark = "before-unresolved-admission";
    let store = SqliteStore::open(&layout.database_path)?;
    store.set_projection_watermark(previous_watermark)?;
    drop(store);

    let result = session.finish(Ok(())).await;
    drop(permit);
    let Some(error) = result.err() else {
        return Err("unfinished submission admission unexpectedly settled".into());
    };
    assert!(
        matches!(
            error.downcast_ref::<sillage_runtime::RuntimeRunError>(),
            Some(sillage_runtime::RuntimeRunError::ShutdownDrainGraceElapsed)
        ),
        "unresolved admission must be reported as drain-grace expiry"
    );
    let store = SqliteStore::open(&layout.database_path)?;
    let watermark = store.projection_watermark()?;
    assert!(
        watermark.as_deref() == Some(previous_watermark),
        "incomplete shutdown must not finalize the projection watermark"
    );
    Ok(())
}

#[test]
fn operation_failure_remains_primary_when_shutdown_also_fails() -> Result<(), &'static str> {
    let Some(error) = super::combine_operation_and_shutdown::<()>(
        Err(anyhow::Error::new(std::io::Error::from(
            std::io::ErrorKind::NotFound,
        ))),
        Err(anyhow::Error::new(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        ))),
    )
    .err() else {
        return Err("both operation and shutdown failures must remain an error");
    };
    if !error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|cause| cause.kind() == std::io::ErrorKind::NotFound)
    {
        return Err("operation error must remain the root failure");
    }
    let shutdown_error = error
        .downcast_ref::<crate::lifecycle::ShutdownFailureContext>()
        .and_then(|context| context.0.downcast_ref::<std::io::Error>())
        .ok_or("shutdown error must remain the original typed failure")?;
    if shutdown_error.kind() != std::io::ErrorKind::PermissionDenied {
        return Err("shutdown failure must preserve its original permission error");
    }
    Ok(())
}
