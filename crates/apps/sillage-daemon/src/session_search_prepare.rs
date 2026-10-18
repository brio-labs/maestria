use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use sillage_domain::SourceChoice;
use sillage_search_tantivy::RamMemoryBudget;
use tokio_util::sync::CancellationToken;

use super::{
    SessionOperationSettlement, SessionSearchRuntime,
    deadline::SessionDeadline,
    ingest,
    resources::{SessionResourceState, SessionResources},
    source_admission::SourceLease,
};

impl SessionSearchRuntime {
    pub(super) async fn prepare_native(
        choice: SourceChoice,
        cancellation: CancellationToken,
        documentary_root: Option<Arc<PathBuf>>,
        source_lease: SourceLease,
        memory: Arc<RamMemoryBudget>,
    ) -> Result<Self> {
        super::validate_session_choice(&choice, documentary_root.is_some())?;
        let deadline = SessionDeadline::after(Duration::from_millis(u64::from(
            choice.budget().max_duration_ms(),
        )));
        let worker_token = cancellation.clone();
        let worker_root = documentary_root;
        let worker_deadline = deadline;
        let worker_lease = source_lease.clone_lease();
        let mut worker = tokio::task::spawn_blocking(move || {
            ingest::prepare(
                choice,
                worker_token,
                memory,
                worker_root,
                worker_deadline,
                worker_lease,
            )
        });
        let prepared = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return Err(joined_interrupted(
                    &mut worker,
                    SessionOperationSettlement::RoutineCancellation,
                ).await);
            }
            _ = tokio::time::sleep(deadline.remaining()) => {
                return Err(joined_interrupted(
                    &mut worker,
                    SessionOperationSettlement::DeadlineExceeded,
                ).await);
            }
            result = &mut worker => settle_worker_result(result),
        }?;
        if cancellation.is_cancelled() {
            return Err(anyhow::Error::new(
                SessionOperationSettlement::RoutineCancellation,
            ));
        }
        if deadline.is_elapsed() {
            return Err(anyhow::Error::new(
                SessionOperationSettlement::DeadlineExceeded,
            ));
        }
        let control = source_lease.control();
        control.ensure_enabled()?;
        let resources = Arc::new(SessionResourceState::new(SessionResources {
            runtime: Arc::new(prepared.runtime),
            _runtime_admission: prepared.runtime_admission,
            stores: prepared.stores,
            retained_sources: prepared.retained_sources,
            retained_generation: prepared.retained_generation,
            _source_lease: Some(prepared.source_lease),
        }));
        control.attach_resources(&resources)?;
        Ok(Self {
            resources,
            choice: prepared.choice,
            selected_path: prepared.selected_path,
            source_root: prepared.source_root,
            manifest: prepared.manifest,
            coverage: prepared.coverage,
            control,
        })
    }
    pub(super) fn commit_retention(&mut self) -> Result<()> {
        self.resources.commit_retention()
    }
}

fn settle_worker_result<T>(
    result: std::result::Result<Result<T>, tokio::task::JoinError>,
) -> Result<T> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(classify_preparation_error(error)),
        Err(error) => {
            Err(anyhow::Error::new(error).context("on-demand source preparation worker failed"))
        }
    }
}

fn classify_preparation_error(error: anyhow::Error) -> anyhow::Error {
    let settlement = match error.downcast_ref::<super::native::NativeError>() {
        Some(super::native::NativeError::Cancelled) => {
            Some(SessionOperationSettlement::RoutineCancellation)
        }
        Some(super::native::NativeError::DeadlineExceeded) => {
            Some(SessionOperationSettlement::DeadlineExceeded)
        }
        _ => None,
    };
    match settlement {
        Some(settlement) => error.context(settlement),
        None => error,
    }
}

async fn joined_interrupted<T>(
    worker: &mut tokio::task::JoinHandle<Result<T>>,
    settlement: SessionOperationSettlement,
) -> anyhow::Error {
    match worker.await {
        Ok(Err(error)) => {
            let error = if error
                .downcast_ref::<super::SessionLifecycleFailure>()
                .is_some()
            {
                error
            } else {
                let error = classify_preparation_error(error);
                if settlement == SessionOperationSettlement::DeadlineExceeded
                    && SessionOperationSettlement::classify_error(&error)
                        == Some(SessionOperationSettlement::RoutineCancellation)
                {
                    error.context(settlement)
                } else {
                    error
                }
            };
            error.context("on-demand source preparation failed while interruption was settling")
        }
        Err(error) => anyhow::Error::new(error)
            .context("on-demand source preparation worker failed while interruption was settling"),
        Ok(Ok(_)) => anyhow::Error::new(settlement),
    }
}
