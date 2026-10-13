use std::{sync::Mutex, time::Duration};

use anyhow::{Result, anyhow};
use sillage_domain::{ArtifactVersionId, EvidenceId};
use tokio_util::sync::CancellationToken;

use super::{
    SessionPreview, SessionSearchRuntime,
    deadline::SessionDeadline,
    ingest,
    preview_verification::{
        PreviewContext, file_span, select_current_preview_source, verify_latest_preview_state,
    },
    resources::SessionResourceSnapshot,
};
use crate::{SearchRuntime, SessionOperationSettlement};

impl SessionSearchRuntime {
    /// Reopen one cited item only while its selected source identity and revision remain current.
    ///
    /// # Cancellation
    /// Cancelling the supplied token or dropping this future requests cancellation. The blocking
    /// verifier is joined before the tracked preview settles.
    pub async fn preview_evidence(
        &self,
        evidence_id: EvidenceId,
        expected_object_version: ArtifactVersionId,
        expected_source_revision: i64,
        cancellation: CancellationToken,
    ) -> Result<SessionPreview> {
        self.ensure_enabled()?;
        let session = self.operation_snapshot();
        let control = session.control.clone();
        let worker_token = cancellation.clone();
        control
            .track(cancellation, async move {
                let _reservation = session.admit_operation(0, 1)?;
                session
                    .preview_admitted(
                        evidence_id,
                        expected_object_version,
                        expected_source_revision,
                        worker_token,
                    )
                    .await
            })
            .await
    }

    async fn preview_admitted(
        &self,
        evidence_id: EvidenceId,
        expected_object_version: ArtifactVersionId,
        expected_source_revision: i64,
        cancellation: CancellationToken,
    ) -> Result<SessionPreview> {
        self.ensure_enabled()?;
        let deadline = SessionDeadline::after(Duration::from_millis(u64::from(
            self.choice.budget().max_duration_ms(),
        )));
        let resources = self.resources.snapshot()?;
        let authorization = self.authorization(&resources.runtime)?;
        let choice = self.choice.clone();
        let selected_path = self.selected_path.clone();
        let manifest = self.manifest.clone();
        let permit = acquire_preview_permit(&resources.runtime, &cancellation, deadline).await?;
        let worker_token = cancellation.clone();
        let mut worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let context = PreviewContext {
                runtime: &resources.runtime,
                stores: &resources.stores,
                choice: &choice,
                selected_path: &selected_path,
                retained_sources: &resources.retained_sources,
                manifest: &manifest,
                authorization: &authorization,
                cancellation: &worker_token,
                deadline,
                verification_work: Mutex::new(0),
            };
            let result = preview_current_evidence(
                &context,
                evidence_id,
                expected_object_version,
                expected_source_revision,
            );
            #[cfg(test)]
            tests::pause_preview_result_gate();
            result
        });
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                cancellation.cancel();
                settle_worker(&mut worker, SessionOperationSettlement::RoutineCancellation).await
            }
            _ = tokio::time::sleep(deadline.remaining()) => {
                settle_worker(&mut worker, SessionOperationSettlement::DeadlineExceeded).await
            }
            result = &mut worker => complete_preview_worker_result(
                result,
                "session evidence preview worker failed",
            )
        };
        let preview = result?;
        self.ensure_enabled()?;
        ensure_preview_active(&cancellation, deadline)?;
        Ok(preview)
    }

    pub(super) async fn open_candidate_batch(
        &self,
        resources: SessionResourceSnapshot,
        candidates: Vec<(EvidenceId, ArtifactVersionId)>,
        expected_source_revision: i64,
        authorization: sillage_governance::RetrievalAuthorizationContext,
        cancellation: CancellationToken,
        deadline: SessionDeadline,
    ) -> Result<(Vec<SessionPreview>, u32)> {
        if candidates.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let choice = self.choice.clone();
        let selected_path = self.selected_path.clone();
        let manifest = self.manifest.clone();
        let permit = acquire_preview_permit(&resources.runtime, &cancellation, deadline).await?;
        let worker_token = cancellation.clone();
        let mut worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let context = PreviewContext {
                runtime: &resources.runtime,
                stores: &resources.stores,
                choice: &choice,
                selected_path: &selected_path,
                retained_sources: &resources.retained_sources,
                manifest: &manifest,
                authorization: &authorization,
                cancellation: &worker_token,
                deadline,
                verification_work: Mutex::new(0),
            };
            let mut previews = Vec::new();
            previews.try_reserve_exact(candidates.len())?;
            let mut stale = 0_u32;
            for (evidence_id, version) in candidates {
                ensure_preview_active(context.cancellation, context.deadline)?;
                match preview_current_evidence(
                    &context,
                    evidence_id,
                    version,
                    expected_source_revision,
                ) {
                    Ok(preview) => previews.push(preview),
                    Err(error) if is_stale_or_denied(&error) => {
                        stale = stale.saturating_add(1);
                    }
                    Err(error) => return Err(error),
                }
                ensure_preview_active(context.cancellation, context.deadline)?;
            }
            Ok((previews, stale))
        });
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                cancellation.cancel();
                settle_worker(&mut worker, SessionOperationSettlement::RoutineCancellation).await
            }
            _ = tokio::time::sleep(deadline.remaining()) => {
                settle_worker(&mut worker, SessionOperationSettlement::DeadlineExceeded).await
            }
            result = &mut worker => complete_preview_worker_result(
                result,
                "session preview batch worker failed",
            )
        };
        let result = result?;
        ensure_preview_active(&cancellation, deadline)?;
        Ok(result)
    }
}

fn complete_preview_worker_result<T>(
    result: std::result::Result<Result<T>, tokio::task::JoinError>,
    error_context: &'static str,
) -> Result<T> {
    let result = result.map_err(|error| anyhow::Error::new(error).context(error_context))?;
    result.map_err(|error| {
        if error
            .downcast_ref::<super::SessionLifecycleFailure>()
            .is_some()
            || SessionOperationSettlement::classify_error(&error).is_some()
        {
            return error;
        }
        match native_preview_settlement(&error) {
            Some(settlement) => error.context(settlement),
            None => error,
        }
    })
}

fn preview_current_evidence(
    context: &PreviewContext<'_>,
    evidence_id: EvidenceId,
    expected_version: ArtifactVersionId,
    expected_source_revision: i64,
) -> Result<SessionPreview> {
    ensure_preview_active(context.cancellation, context.deadline)?;
    if context.runtime.event_log.searchable_source_revision()? != expected_source_revision {
        return Err(super::StaleEvidence("session source revision is no longer current").into());
    }
    let (_, sources) = context.runtime.interactive_current_sources()?;
    let evidence = context
        .stores
        .evidence(evidence_id)?
        .ok_or(super::StaleEvidence("session evidence does not exist"))?;
    let selected = select_current_preview_source(context, &sources, &evidence, expected_version)?;
    let source_validation = ingest::RetainedSourceValidation {
        binding: selected.binding,
        max_bytes: context.choice.budget().max_file_bytes(),
        expected_hash: &selected.hash,
        cancellation: context.cancellation,
        deadline: context.deadline,
        memory: &context.stores.memory,
        max_work: context.choice.budget().max_work_units(),
        work: &context.verification_work,
    };
    ingest::verify_retained_source(&source_validation)?;
    ensure_preview_active(context.cancellation, context.deadline)?;
    let output = context.stores.open_evidence(ingest::EvidenceOpenContext {
        evidence_id,
        authorization: context.authorization,
        cancellation: context.cancellation,
        deadline: context.deadline,
        max_file_bytes: context.choice.budget().max_file_bytes(),
        max_work_units: context.choice.budget().max_work_units(),
        verification_work: &context.verification_work,
    })?;
    if output.evidence.artifact_id != selected.evidence.artifact_id {
        return Err(super::StaleEvidence(
            "session evidence ownership changed while opening its preview",
        )
        .into());
    }
    let (opened_path, opened_range, opened_snapshot) = file_span(&output.evidence).ok_or(
        super::StaleEvidence("reopened session evidence is not a Markdown file span"),
    )?;
    if selected.path.to_str() != Some(opened_path)
        || opened_range != selected.evidence_range
        || opened_snapshot.content_hash() != &selected.hash
    {
        return Err(super::StaleEvidence(
            "session evidence locator or snapshot changed while opening its preview",
        )
        .into());
    }
    ingest::verify_retained_source(&source_validation)?;
    ensure_preview_active(context.cancellation, context.deadline)?;
    let source_revision =
        verify_latest_preview_state(context, &selected, &output, expected_source_revision)?;
    let truncated = context.stores.excerpt_was_truncated(evidence_id)?;
    Ok(SessionPreview {
        evidence_id,
        artifact_id: output.artifact.id,
        artifact_version: selected.version,
        source_path: selected.path.to_path_buf(),
        content_hash: selected.hash,
        line_range: *opened_range,
        source_revision,
        excerpt: output.evidence.excerpt,
        truncated,
    })
}

async fn settle_worker<T>(
    worker: &mut tokio::task::JoinHandle<Result<T>>,
    interruption: SessionOperationSettlement,
) -> Result<T> {
    match worker.await {
        Ok(Err(error)) => {
            if error
                .downcast_ref::<super::SessionLifecycleFailure>()
                .is_some()
            {
                return Err(error);
            }
            match SessionOperationSettlement::classify_error(&error) {
                Some(SessionOperationSettlement::RoutineCancellation)
                    if interruption == SessionOperationSettlement::DeadlineExceeded =>
                {
                    Err(error.context(interruption))
                }
                Some(_) => Err(error),
                None => match native_preview_settlement(&error) {
                    Some(native)
                        if native == SessionOperationSettlement::DeadlineExceeded
                            || interruption == SessionOperationSettlement::DeadlineExceeded =>
                    {
                        Err(error.context(SessionOperationSettlement::DeadlineExceeded))
                    }
                    Some(_) => Err(error.context(SessionOperationSettlement::RoutineCancellation)),
                    None => Err(error),
                },
            }
        }
        Err(error) => Err(anyhow::Error::new(error).context("session preview worker failed")),
        Ok(Ok(_)) => Err(anyhow::Error::new(interruption)),
    }
}

fn native_preview_settlement(error: &anyhow::Error) -> Option<SessionOperationSettlement> {
    match error.downcast_ref::<super::native::NativeError>() {
        Some(super::native::NativeError::Cancelled) => {
            Some(SessionOperationSettlement::RoutineCancellation)
        }
        Some(super::native::NativeError::DeadlineExceeded) => {
            Some(SessionOperationSettlement::DeadlineExceeded)
        }
        _ => None,
    }
}

fn is_stale_or_denied(error: &anyhow::Error) -> bool {
    if error.downcast_ref::<super::StaleEvidence>().is_some() {
        return true;
    }
    matches!(
        error.downcast_ref::<sillage_core::CoreError>(),
        Some(
            sillage_core::CoreError::NotFound { .. }
                | sillage_core::CoreError::NotFoundEntity { .. }
                | sillage_core::CoreError::NotAvailable { .. }
                | sillage_core::CoreError::RetrievalAuthorization(_)
                | sillage_core::CoreError::Port(sillage_ports::PortError::NotFound)
        )
    )
}
async fn acquire_preview_permit(
    runtime: &SearchRuntime,
    cancellation: &CancellationToken,
    deadline: SessionDeadline,
) -> Result<tokio::sync::OwnedSemaphorePermit> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            Err(SessionOperationSettlement::RoutineCancellation.into())
        }
        _ = tokio::time::sleep(deadline.remaining()) => {
            Err(SessionOperationSettlement::DeadlineExceeded.into())
        }
        permit = runtime.interactive_search_workers.clone().acquire_owned() => {
            permit.map_err(|error| anyhow!("interactive search worker pool closed: {error}"))
        }
    }
}
fn ensure_preview_active(
    cancellation: &CancellationToken,
    deadline: SessionDeadline,
) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(SessionOperationSettlement::RoutineCancellation.into());
    }
    if deadline.is_elapsed() {
        return Err(SessionOperationSettlement::DeadlineExceeded.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_search_preview_tests.rs"]
mod tests;
