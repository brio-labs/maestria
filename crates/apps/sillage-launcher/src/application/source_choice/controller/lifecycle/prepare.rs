use std::path::PathBuf;
use std::sync::Arc;

use sillage_daemon::{
    SessionOperationSettlement, SessionPreparationOwner, SessionSearchCancellation,
    SessionSearchRuntime,
};
use sillage_domain::{ControlPersistenceChoice, DocumentaryRetentionChoice, SourceChoice};

#[cfg(test)]
use super::super::super::state::LifecycleTestStage;
use super::super::super::state::{SessionSourceState, TerminalOperationReservation};
use super::super::{build_choice, retained_root};
use crate::application::platform::validate_markdown_file;

pub(super) async fn prepare_source(state: &SessionSourceState) {
    let (epoch, selected_path, retention, persistence, cancellation, terminal_slot) =
        match state.begin_prepare() {
            Ok(preparation) => preparation,
            Err(error) => {
                state.set_error(error);
                return;
            }
        };
    #[cfg(test)]
    state.wait_at_prepare_test_gate().await;
    let Some((choice, owner, terminal_slot)) = prepare_source_input(
        state,
        epoch,
        selected_path,
        retention,
        persistence,
        &cancellation,
        terminal_slot,
    ) else {
        return;
    };
    let owner = Arc::new(owner);
    match owner
        .prepare_cancellable(choice.clone(), cancellation.clone())
        .await
    {
        Ok(runtime) if cancellation.is_cancelled() => {
            let settlement = settle_unpublished(owner, Arc::new(runtime)).await;
            match settlement {
                Ok(()) => state.settle_prepare_failure(
                    epoch,
                    &cancellation,
                    "Preparation cancelled; owned work was settled".to_string(),
                    true,
                    false,
                    terminal_slot,
                ),
                Err(error) => state.settle_prepare_failure(
                    epoch,
                    &cancellation,
                    format!("Preparation was cancelled; settlement failed: {error}"),
                    false,
                    true,
                    terminal_slot,
                ),
            }
        }
        Ok(runtime) => {
            let runtime = Arc::new(runtime);
            if !state.finish_prepare(
                epoch,
                &cancellation,
                Arc::clone(&owner),
                Arc::clone(&runtime),
                choice,
            ) {
                let cancelled = cancellation.is_cancelled();
                let settlement = settle_unpublished(owner, runtime).await;
                let (error, routine, retain_critical_failure) = match settlement {
                    Ok(()) => (
                        if cancelled {
                            "Preparation cancelled; owned work was settled".to_string()
                        } else {
                            "Preparation superseded; owned work was settled".to_string()
                        },
                        true,
                        false,
                    ),
                    Err(error) => (
                        format!("Unpublished source cleanup failed: {error}"),
                        false,
                        true,
                    ),
                };
                state.settle_prepare_failure(
                    epoch,
                    &cancellation,
                    error,
                    routine,
                    retain_critical_failure,
                    terminal_slot,
                );
            }
        }
        Err(error) => {
            let routine = SessionOperationSettlement::classify_error(&error).is_some();
            settle_prepare_failure(
                state,
                epoch,
                owner,
                cancellation,
                terminal_slot,
                format!("{error:#}"),
                routine,
            )
            .await;
        }
    }
}

fn prepare_source_input<'a>(
    state: &'a SessionSourceState,
    epoch: u64,
    selected_path: PathBuf,
    retention: DocumentaryRetentionChoice,
    persistence: ControlPersistenceChoice,
    cancellation: &SessionSearchCancellation,
    terminal_slot: TerminalOperationReservation<'a>,
) -> Option<(
    SourceChoice,
    SessionPreparationOwner,
    TerminalOperationReservation<'a>,
)> {
    let path = match validate_markdown_file(&selected_path) {
        Ok(path) if path == selected_path => path,
        Ok(_) => {
            state.settle_prepare_failure(
                epoch,
                cancellation,
                concat!(
                    "The selected Markdown path now resolves to a different file; ",
                    "choose it again and re-authorize reading.",
                )
                .to_string(),
                false,
                false,
                terminal_slot,
            );
            return None;
        }
        Err(error) => {
            state.settle_prepare_failure(
                epoch,
                cancellation,
                error.message,
                false,
                false,
                terminal_slot,
            );
            return None;
        }
    };
    let choice = match build_choice(path.clone(), retention, persistence) {
        Ok(choice) => choice,
        Err(error) => {
            state.settle_prepare_failure(epoch, cancellation, error, false, false, terminal_slot);
            return None;
        }
    };
    let owner = match retention {
        DocumentaryRetentionChoice::NoDocumentaryPersistence => SessionPreparationOwner::new(),
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged => {
            let root = match retained_root(&path) {
                Ok(root) => root,
                Err(error) => {
                    state.settle_prepare_failure(
                        epoch,
                        cancellation,
                        error,
                        false,
                        false,
                        terminal_slot,
                    );
                    return None;
                }
            };
            match SessionPreparationOwner::with_retention_root(root) {
                Ok(owner) => owner,
                Err(error) => {
                    state.settle_prepare_failure(
                        epoch,
                        cancellation,
                        format!("{error:#}"),
                        false,
                        false,
                        terminal_slot,
                    );
                    return None;
                }
            }
        }
    };
    Some((choice, owner, terminal_slot))
}

async fn settle_unpublished(
    owner: Arc<SessionPreparationOwner>,
    runtime: Arc<SessionSearchRuntime>,
) -> Result<(), String> {
    let runtime_result = runtime.retire().await;
    runtime.wait_for_idle().await;
    let owner_result = owner.retire().await;
    owner.wait_for_idle().await;
    match (runtime_result, owner_result) {
        (Ok(()), Ok(())) => Ok(()),
        (runtime, owner) => Err(format!(
            "runtime settlement: {}; preparation-owner settlement: {}",
            runtime.map_or_else(|error| format!("{error:#}"), |_| "settled".to_string()),
            owner.map_or_else(|error| format!("{error:#}"), |_| "settled".to_string()),
        )),
    }
}

pub(super) async fn settle_prepare_failure(
    state: &SessionSourceState,
    epoch: u64,
    owner: Arc<SessionPreparationOwner>,
    cancellation: SessionSearchCancellation,
    terminal_slot: TerminalOperationReservation<'_>,
    original_error: String,
    routine_settlement: bool,
) {
    let settled = owner.retire().await;
    owner.wait_for_idle().await;
    #[cfg(test)]
    state.wait_at_lifecycle_test_gate(
        LifecycleTestStage::PrepareBeforeAtomicFailureSettlement,
        !routine_settlement,
        Some(original_error.clone()),
    );
    match settled {
        Ok(()) => state.settle_prepare_failure(
            epoch,
            &cancellation,
            original_error,
            routine_settlement,
            false,
            terminal_slot,
        ),
        Err(settle_error) => {
            let error = format!("{original_error}; owner settlement failed: {settle_error:#}");
            state.settle_prepare_failure(epoch, &cancellation, error, false, true, terminal_slot);
        }
    }
}
