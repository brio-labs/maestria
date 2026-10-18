#[cfg(unix)]
use std::path::PathBuf;
use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use sillage_domain::{
    ComputationTiming, ControlPersistenceChoice, DocumentaryRetentionChoice, SourceAvailability,
    SourceChoice, SourceChoiceParameters, SourceLocality, SourceRepresentation,
    SourceResourceBudget, SourceScope,
};

use super::{PREVIEW_GATE_SERIAL, PreviewWorkspaceGuard, create_workspace, selected_choice};
#[cfg(unix)]
use tokio_util::sync::CancellationToken;

struct PreviewObservation {
    reached: bool,
    result: Result<crate::SessionPreview>,
}

#[tokio::test]
async fn preview_native_cancellation_keeps_typed_settlement_and_original_cause() -> Result<()> {
    let _serial = PREVIEW_GATE_SERIAL.lock().await;
    let (root, source) = create_workspace()?;
    let workspace = PreviewWorkspaceGuard::new(root);
    let owner = crate::SessionPreparationOwner::new();
    let runtime = Arc::new(
        owner
            .prepare_cancellable(
                selected_choice(
                    source.clone(),
                    DocumentaryRetentionChoice::NoDocumentaryPersistence,
                )?,
                crate::SessionSearchCancellation::new(),
            )
            .await
            .context("cannot prepare the native-cancellation fixture")?,
    );
    let cancellation = crate::SessionSearchCancellation::new();
    let observation = preview_at_native_acquisition(&runtime, &source, cancellation, false).await?;
    let classified = observation.result.as_ref().err().is_some_and(|error| {
        crate::SessionOperationSettlement::classify_error(error)
            == Some(crate::SessionOperationSettlement::RoutineCancellation)
    });
    let original_cause = observation.result.as_ref().err().is_some_and(|error| {
        error.chain().any(|cause| {
            matches!(
                cause.downcast_ref::<crate::session_search::native::NativeError>(),
                Some(crate::session_search::native::NativeError::Cancelled)
            )
        })
    });
    let runtime_retirement = runtime.retire().await;
    let owner_retirement = owner.retire().await;
    workspace.cleanup()?;
    if !observation.reached || !classified || !original_cause {
        bail!(
            "preview cancellation classification failed (native checkpoint reached: {}; \
             typed routine settlement: {}; original native cause preserved: {})",
            observation.reached,
            classified,
            original_cause
        );
    }
    runtime_retirement.context("native-cancelled preview runtime cleanup failed")?;
    owner_retirement.context("native-cancelled preview owner cleanup failed")?;
    Ok(())
}

#[tokio::test]
async fn preview_native_deadline_keeps_typed_settlement_and_original_cause() -> Result<()> {
    let _serial = PREVIEW_GATE_SERIAL.lock().await;
    let (root, source) = create_workspace()?;
    let workspace = PreviewWorkspaceGuard::new(root);
    let owner = crate::SessionPreparationOwner::new();
    let choice = choice_with_duration(&source, 5_000)?;
    let runtime = Arc::new(
        owner
            .prepare_cancellable(choice, crate::SessionSearchCancellation::new())
            .await
            .context("cannot prepare the native-deadline fixture")?,
    );
    let observation = preview_at_native_acquisition(
        &runtime,
        &source,
        crate::SessionSearchCancellation::new(),
        true,
    )
    .await?;
    let classified = observation.result.as_ref().err().is_some_and(|error| {
        crate::SessionOperationSettlement::classify_error(error)
            == Some(crate::SessionOperationSettlement::DeadlineExceeded)
    });
    let original_cause = observation.result.as_ref().err().is_some_and(|error| {
        error.chain().any(|cause| {
            matches!(
                cause.downcast_ref::<crate::session_search::native::NativeError>(),
                Some(crate::session_search::native::NativeError::DeadlineExceeded)
            )
        })
    });
    let runtime_retirement = runtime.retire().await;
    let owner_retirement = owner.retire().await;
    workspace.cleanup()?;
    if !observation.reached || !classified || !original_cause {
        bail!(
            "preview deadline classification failed (native checkpoint reached: {}; \
             typed deadline settlement: {}; original native cause preserved: {})",
            observation.reached,
            classified,
            original_cause
        );
    }
    runtime_retirement.context("native-deadline preview runtime cleanup failed")?;
    owner_retirement.context("native-deadline preview owner cleanup failed")?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn completed_native_deadline_errors_use_normalized_preview_worker_path() -> Result<()> {
    let _serial = PREVIEW_GATE_SERIAL.lock().await;
    let (root, source) = create_workspace()?;
    let workspace = PreviewWorkspaceGuard::new(root);
    let preview_completed = native_deadline_worker(source.clone()).await;
    let batch_completed = native_deadline_worker(source.clone()).await;
    let preview_native_error = contains_native_deadline(&preview_completed);
    let batch_native_error = contains_native_deadline(&batch_completed);
    let preview_result = super::super::complete_preview_worker_result(
        preview_completed,
        "session evidence preview worker failed",
    );
    let batch_result = super::super::complete_preview_worker_result(
        batch_completed,
        "session preview batch worker failed",
    );
    let preview_typed = preview_result.as_ref().err().is_some_and(|error| {
        crate::SessionOperationSettlement::classify_error(error)
            == Some(crate::SessionOperationSettlement::DeadlineExceeded)
    });
    let batch_typed = batch_result.as_ref().err().is_some_and(|error| {
        crate::SessionOperationSettlement::classify_error(error)
            == Some(crate::SessionOperationSettlement::DeadlineExceeded)
    });
    let preview_cause_preserved = preview_result
        .as_ref()
        .err()
        .is_some_and(contains_native_deadline_error);
    let batch_cause_preserved = batch_result
        .as_ref()
        .err()
        .is_some_and(contains_native_deadline_error);
    workspace.cleanup()?;
    if !preview_native_error
        || !batch_native_error
        || !preview_typed
        || !batch_typed
        || !preview_cause_preserved
        || !batch_cause_preserved
    {
        bail!(
            "completed preview workers omitted native deadline normalization (native causes \
             preview/batch: {preview_native_error}/{batch_native_error}; typed preview/batch: \
             {preview_typed}/{batch_typed}; causes preserved preview/batch: \
             {preview_cause_preserved}/{batch_cause_preserved})"
        );
    }
    Ok(())
}

#[cfg(unix)]
fn native_deadline_worker(path: PathBuf) -> tokio::task::JoinHandle<Result<()>> {
    tokio::task::spawn_blocking(move || {
        let memory = Arc::new(sillage_search_tantivy::RamMemoryBudget::new(16));
        let cancellation = CancellationToken::new();
        let admission = crate::session_search::retained::Admission {
            memory: &memory,
            cancellation: &cancellation,
            deadline: crate::session_search::deadline::SessionDeadline::after(Duration::ZERO),
        };
        crate::session_search::native::acquire_file(&path, &admission)
            .map(|_file| ())
            .map_err(anyhow::Error::new)
    })
}

#[cfg(unix)]
fn contains_native_deadline(
    result: &std::result::Result<Result<()>, tokio::task::JoinError>,
) -> bool {
    matches!(result, Ok(Err(error)) if contains_native_deadline_error(error))
}

#[cfg(unix)]
fn contains_native_deadline_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<crate::session_search::native::NativeError>(),
            Some(crate::session_search::native::NativeError::DeadlineExceeded)
        )
    })
}

async fn preview_at_native_acquisition(
    runtime: &Arc<crate::SessionSearchRuntime>,
    source: &Path,
    cancellation: crate::SessionSearchCancellation,
    wait_for_deadline: bool,
) -> Result<PreviewObservation> {
    let response = runtime
        .search("saffron comet".to_string(), 8)
        .await
        .context("cannot establish the selected-source citation")?;
    let citation = response
        .previews
        .first()
        .ok_or_else(|| anyhow::anyhow!("selected-source fixture produced no citation"))?
        .clone();
    let (gate, reached_signal) = crate::session_search::native::NativeAcquireTestGate::new(source);
    crate::session_search::native::NativeAcquireTestGate::install(gate.clone())?;
    let worker_runtime = runtime.clone();
    let worker_cancellation = cancellation.clone();
    let worker = tokio::spawn(async move {
        worker_runtime
            .preview_evidence(
                citation.evidence_id,
                citation.artifact_version,
                citation.source_revision,
                worker_cancellation,
            )
            .await
    });
    let reached = tokio::time::timeout(Duration::from_secs(10), reached_signal)
        .await
        .is_ok_and(|signal| signal.is_ok());
    if reached && wait_for_deadline {
        let deadline =
            Duration::from_millis(u64::from(runtime.choice().budget().max_duration_ms()))
                .saturating_add(Duration::from_millis(25));
        tokio::time::sleep(deadline).await;
    } else if !wait_for_deadline || !reached {
        cancellation.cancel();
    }
    gate.release();
    let joined = worker
        .await
        .map_err(anyhow::Error::new)
        .context("native-interrupted preview caller failed to join");
    crate::session_search::native::NativeAcquireTestGate::clear(&gate);
    Ok(PreviewObservation {
        reached,
        result: joined?,
    })
}

fn choice_with_duration(source: &Path, max_duration_ms: u32) -> Result<SourceChoice> {
    let defaults = SourceResourceBudget::session_default();
    let budget = SourceResourceBudget::try_new(
        defaults.max_entries(),
        defaults.max_file_bytes(),
        defaults.max_total_source_bytes(),
        defaults.max_in_memory_bytes(),
        defaults.max_results(),
        defaults.max_work_units(),
        max_duration_ms,
    )?;
    SourceChoice::try_new(SourceChoiceParameters {
        scope: SourceScope::file(source.to_path_buf())?,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention: DocumentaryRetentionChoice::NoDocumentaryPersistence,
        control_persistence: ControlPersistenceChoice::SessionOnly,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::LexicalIndex],
        remote_disclosures: Vec::new(),
        budget,
    })
    .map_err(anyhow::Error::new)
}
