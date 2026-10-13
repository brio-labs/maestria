use std::{future::Future, path::PathBuf, sync::Arc, task::Poll, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use sillage_domain::DocumentaryRetentionChoice;

use super::{
    PREVIEW_GATE_SERIAL, PreviewTestGate, PreviewWorkspaceGuard, clear_preview_result_gate,
    create_workspace, install_preview_result_gate, selected_choice,
};
use crate::session_search::ingest::{
    PARSE_TEST_SERIAL, ParseCheckpointGate, clear_gate as clear_parse_gate,
    install_gate as install_parse_gate,
};

#[tokio::test]
async fn dropped_pending_retirement_releases_resources_after_accepted_preview_and_idle()
-> Result<()> {
    let _serial = PREVIEW_GATE_SERIAL.lock().await;
    let fixture = RetirementFixture::prepare().await?;
    let observations = observe_dropped_retirement(&fixture).await?;
    let checks = observations.checks;
    fixture.cleanup(observations).await?;
    checks.require_success()
}

#[tokio::test]
async fn dropped_owner_retire_preserves_preparation_error_after_gated_preview() -> Result<()> {
    let _preview_serial = PREVIEW_GATE_SERIAL.lock().await;
    let _parse_serial = PARSE_TEST_SERIAL.lock().await;
    let (root, source) = create_workspace()?;
    let workspace = PreviewWorkspaceGuard::new(root);
    let owner =
        crate::SessionPreparationOwner::with_retention_root(workspace.root.join("retained"))?;
    let runtime = Arc::new(
        owner
            .prepare_cancellable(
                selected_choice(
                    source.clone(),
                    DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
                )?,
                crate::SessionSearchCancellation::new(),
            )
            .await
            .context("cannot prepare the owner-error fixture")?,
    );

    let failing_source = workspace.root.join("dropped-preparation.md");
    std::fs::copy(&source, &failing_source)?;
    let failing_choice = selected_choice(
        failing_source.clone(),
        DocumentaryRetentionChoice::NoDocumentaryPersistence,
    )?;
    let (parse_gate, parse_reached_signal) = ParseCheckpointGate::new(failing_source);
    install_parse_gate(parse_gate.clone())?;
    let failing_owner = owner.clone();
    let failing_preparation = tokio::spawn(async move {
        failing_owner
            .prepare_cancellable(failing_choice, crate::SessionSearchCancellation::new())
            .await
    });
    let preparation_reached = tokio::time::timeout(Duration::from_secs(10), parse_reached_signal)
        .await
        .is_ok_and(|result| result.is_ok());
    failing_preparation.abort();
    parse_gate.release();
    let caller_was_dropped =
        matches!(failing_preparation.await, Err(error) if error.is_cancelled());
    clear_parse_gate(&parse_gate);
    owner.wait_for_idle().await;

    let response = runtime
        .search("saffron comet".to_string(), 8)
        .await
        .context("cannot establish the preview citation")?;
    let citation = response
        .previews
        .first()
        .ok_or_else(|| anyhow!("owner-error fixture produced no citation"))?
        .clone();
    let (preview_gate, preview_reached_signal) = PreviewTestGate::new();
    install_preview_result_gate(preview_gate.clone())?;
    let worker_runtime = runtime.clone();
    let preview_worker = tokio::spawn(async move {
        worker_runtime
            .preview_evidence(
                citation.evidence_id,
                citation.artifact_version,
                citation.source_revision,
                crate::SessionSearchCancellation::new(),
            )
            .await
    });
    let preview_reached = tokio::time::timeout(Duration::from_secs(10), preview_reached_signal)
        .await
        .is_ok_and(|result| result.is_ok());
    let mut retirement = Box::pin(owner.retire());
    let retirement_poll = if preview_reached {
        Some(std::future::poll_fn(|context| Poll::Ready(retirement.as_mut().poll(context))).await)
    } else {
        None
    };
    drop(retirement);
    preview_gate.release();
    let preview_joined = preview_worker
        .await
        .map_err(anyhow::Error::new)
        .context("accepted preview caller failed to join");
    clear_preview_result_gate(&preview_gate);
    owner.wait_for_idle().await;

    let recovered_retirement = owner.retire().await;
    let later_settlement = recovered_retirement.as_ref().err().is_some_and(|error| {
        crate::SessionOperationSettlement::classify_error(error)
            == Some(crate::SessionOperationSettlement::RoutineCancellation)
    });
    let original_parser_cause = recovered_retirement.as_ref().err().is_some_and(|error| {
        error.chain().any(|cause| {
            matches!(
                cause.downcast_ref::<sillage_ports::BoundedParseError>(),
                Some(sillage_ports::BoundedParseError::Budget(
                    sillage_ports::ParseBudgetFailure::Cancelled
                ))
            )
        })
    });
    let recovered_again = owner.retire().await;
    let original_reported_once = !recovered_again.as_ref().err().is_some_and(|error| {
        error.chain().any(|cause| {
            matches!(
                cause.downcast_ref::<sillage_ports::BoundedParseError>(),
                Some(sillage_ports::BoundedParseError::Budget(
                    sillage_ports::ParseBudgetFailure::Cancelled
                ))
            )
        })
    });
    let preview_cancelled = preview_joined.as_ref().ok().is_some_and(|result| {
        result.as_ref().err().is_some_and(|error| {
            crate::SessionOperationSettlement::classify_error(error)
                == Some(crate::SessionOperationSettlement::RoutineCancellation)
        })
    });

    let runtime_retirement = runtime.retire().await;
    let owner_retirement = owner.retire().await;
    let purge = owner.purge_retained_source(&source);
    let workspace_cleanup = workspace.cleanup();
    runtime_retirement.context("owner-error runtime cleanup failed")?;
    owner_retirement.context("owner-error owner cleanup failed")?;
    purge.context("owner-error retained-source purge failed")?;
    workspace_cleanup?;
    if !preparation_reached
        || !caller_was_dropped
        || !preview_reached
        || !matches!(retirement_poll, Some(Poll::Pending))
        || !preview_cancelled
        || !later_settlement
        || !original_parser_cause
        || !original_reported_once
    {
        bail!(
            "dropped owner retirement lost accepted preparation errors (parse gate reached: {}; \
             caller dropped: {}; preview gate reached: {}; retire pending before drop: {}; \
             preview cancellation settled: {}; later typed settlement: {}; original parser cause \
             preserved: {}; original reported once: {})",
            preparation_reached,
            caller_was_dropped,
            preview_reached,
            matches!(retirement_poll, Some(Poll::Pending)),
            preview_cancelled,
            later_settlement,
            original_parser_cause,
            original_reported_once,
        );
    }
    Ok(())
}

struct RetirementFixture {
    workspace: PreviewWorkspaceGuard,
    source: PathBuf,
    owner: crate::SessionPreparationOwner,
    runtime: Arc<crate::SessionSearchRuntime>,
}

impl RetirementFixture {
    async fn prepare() -> Result<Self> {
        let (root, source) = create_workspace()?;
        let workspace = PreviewWorkspaceGuard::new(root);
        let owner =
            crate::SessionPreparationOwner::with_retention_root(workspace.root.join("retained"))?;
        let runtime = Arc::new(
            owner
                .prepare_cancellable(
                    selected_choice(
                        source.clone(),
                        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
                    )?,
                    crate::SessionSearchCancellation::new(),
                )
                .await
                .context("cannot prepare the retained Markdown fixture")?,
        );
        Ok(Self {
            workspace,
            source,
            owner,
            runtime,
        })
    }

    async fn cleanup(self, observations: RetirementObservations) -> Result<()> {
        let replacement_retirement = match observations.replacement_runtime {
            Some(replacement) => replacement.retire().await,
            None => Ok(()),
        };
        let replacement_owner_retirement = observations.replacement_owner.retire().await;
        let runtime_retirement = self.runtime.retire().await;
        let owner_retirement = self.owner.retire().await;
        let purge = self.owner.purge_retained_source(&self.source);
        replacement_retirement.context("replacement runtime cleanup failed")?;
        replacement_owner_retirement.context("replacement owner cleanup failed")?;
        runtime_retirement.context("runtime cleanup after regression observations failed")?;
        owner_retirement.context("owner cleanup after regression observations failed")?;
        purge.context("retained-source cleanup after regression observations failed")?;
        self.workspace.cleanup()
    }
}

#[derive(Clone, Copy)]
struct RetirementChecks {
    reached: bool,
    pending: bool,
    worker_cancelled: bool,
    resources_released: bool,
    purge_released: bool,
    source_replaced: bool,
}

impl RetirementChecks {
    fn require_success(self) -> Result<()> {
        if !self.reached
            || !self.pending
            || !self.worker_cancelled
            || !self.resources_released
            || !self.purge_released
            || !self.source_replaced
        {
            bail!(
                "dropped retirement failed (accepted preview reached gate: {}; \
                 retirement was pending before drop: {}; caller saw typed cancellation: {}; \
                 resources released after idle: {}; purge without another retire: {}; \
                 source replaceable without another retire: {})",
                self.reached,
                self.pending,
                self.worker_cancelled,
                self.resources_released,
                self.purge_released,
                self.source_replaced
            );
        }
        Ok(())
    }
}

struct RetirementObservations {
    checks: RetirementChecks,
    replacement_owner: crate::SessionPreparationOwner,
    replacement_runtime: Option<crate::SessionSearchRuntime>,
}

async fn observe_dropped_retirement(fixture: &RetirementFixture) -> Result<RetirementObservations> {
    let response = fixture
        .runtime
        .search("saffron comet".to_string(), 8)
        .await
        .context("cannot establish the selected-source citation")?;
    let citation = response
        .previews
        .first()
        .ok_or_else(|| anyhow!("selected-source fixture produced no citation"))?
        .clone();
    let (gate, reached_signal) = PreviewTestGate::new();
    install_preview_result_gate(gate.clone())?;
    let cancellation = crate::SessionSearchCancellation::new();
    let worker_runtime = fixture.runtime.clone();
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
        .is_ok_and(|result| result.is_ok());
    let mut retirement = Box::pin(fixture.runtime.retire());
    let retirement_poll = if reached {
        Some(std::future::poll_fn(|context| Poll::Ready(retirement.as_mut().poll(context))).await)
    } else {
        None
    };
    drop(retirement);
    gate.release();
    let joined = worker
        .await
        .map_err(anyhow::Error::new)
        .context("accepted preview caller failed to join");
    clear_preview_result_gate(&gate);
    let worker_result = joined?;
    fixture.runtime.wait_for_idle().await;
    let resources_released = fixture.runtime.resources.snapshot().is_err();
    let purge_released = fixture.owner.purge_retained_source(&fixture.source).is_ok();

    let replacement_owner = crate::SessionPreparationOwner::new();
    let replacement = replacement_owner
        .prepare_cancellable(
            selected_choice(
                fixture.source.clone(),
                DocumentaryRetentionChoice::NoDocumentaryPersistence,
            )?,
            crate::SessionSearchCancellation::new(),
        )
        .await;
    let source_replaced = replacement.is_ok();
    let worker_cancelled = worker_result.as_ref().err().is_some_and(|error| {
        crate::SessionOperationSettlement::classify_error(error)
            == Some(crate::SessionOperationSettlement::RoutineCancellation)
    });
    Ok(RetirementObservations {
        checks: RetirementChecks {
            reached,
            pending: matches!(retirement_poll, Some(Poll::Pending)),
            worker_cancelled,
            resources_released,
            purge_released,
            source_replaced,
        },
        replacement_owner,
        replacement_runtime: replacement.ok(),
    })
}
