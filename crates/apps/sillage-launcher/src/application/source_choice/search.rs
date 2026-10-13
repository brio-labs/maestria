use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::oneshot;

use super::response;
#[cfg(test)]
use super::state::LifecycleTestStage;
use super::state::{SessionSourceState, TerminalOperationReservation};
use crate::application::{Frontend, UiWeak};

pub(super) async fn run_query_worker(
    state: Arc<SessionSourceState>,
    frontend: Arc<Frontend>,
    ui: UiWeak,
) -> Result<(), String> {
    let mut requests = state.subscribe_queries();
    loop {
        if state.is_stopping() {
            return Ok(());
        }
        if requests.changed().await.is_err() {
            return Ok(());
        }
        if state.is_stopping() {
            return Ok(());
        }
        let request = requests.borrow_and_update().clone();
        let Some(request) = request else {
            continue;
        };
        let reservation = match state.begin_query_operation(&request) {
            Ok(Some(reservation)) => reservation,
            Ok(None) => continue,
            Err(error) => {
                if state.query_request_is_current(&request) {
                    state.fail_query(&request, error.clone(), false);
                    show_query_error(&ui, &state, &request, error);
                }
                continue;
            }
        };
        let terminal_slot = reservation;
        if let Err(error) = run_query(&state, &frontend, &ui, &request).await {
            settle_query_failure(&state, &ui, &request, terminal_slot, error);
        }
        state.finish_query_operation(&request);
    }
}

fn settle_query_failure(
    state: &Arc<SessionSourceState>,
    ui: &UiWeak,
    request: &super::state::SourceQuery,
    terminal_slot: TerminalOperationReservation<'_>,
    error: response::SourceOperationFailure,
) {
    let routine_settlement = error.is_routine_settlement();
    let message = error.message().to_string();

    #[cfg(test)]
    state.wait_at_lifecycle_test_gate(
        LifecycleTestStage::QueryBeforeAtomicFailureSettlement,
        !routine_settlement,
        Some(message.clone()),
    );
    if state.settle_query_failure(
        request,
        message.clone(),
        false,
        routine_settlement,
        terminal_slot,
    ) {
        show_query_error(ui, state, request, message);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum QueryOutcome {
    Applied,
    Cancelled,
}

async fn run_query(
    state: &Arc<SessionSourceState>,
    frontend: &Arc<Frontend>,
    ui: &UiWeak,
    request: &super::state::SourceQuery,
) -> Result<QueryOutcome, response::SourceOperationFailure> {
    let readiness = request.catalog_applied.lock().await.take().ok_or_else(|| {
        "The selected-source query lost its foreground catalog result".to_string()
    })?;
    let ready = tokio::select! {
        biased;
        _ = request.cancellation.cancelled() => false,
        result = readiness => result.is_ok(),
    };
    if !ready || !state.query_is_current(request) {
        return Ok(QueryOutcome::Cancelled);
    }
    let Some((runtime, selected_path, _)) = state.current_runtime_for_query(request) else {
        return Ok(QueryOutcome::Cancelled);
    };
    let Some(mapped) = response::search(&runtime, &selected_path, request, state).await? else {
        return Ok(QueryOutcome::Cancelled);
    };
    if !state.query_is_current(request) {
        return Ok(QueryOutcome::Cancelled);
    }
    let revision = mapped.source_revision;
    let phase = mapped.phase;
    let status = mapped.status;
    let detail = mapped.detail;
    let result = mapped.passages;
    let (applied, wait_for_application) = oneshot::channel();
    let ui = ui.clone();
    let frontend = Arc::clone(frontend);
    let state = Arc::clone(state);
    let cancellation = request.cancellation.clone();
    let request = request.clone();
    let generation = request.generation;
    let query = request.query.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if state.query_is_current(&request)
            && frontend.generation.load(Ordering::Acquire) == generation
            && let Some(window) = ui.upgrade()
        {
            crate::application::search::apply_passages(
                &window, &frontend, generation, &query, result, false,
            );
            window.set_index_status(status.clone().into());
            window.set_status_message(status.clone().into());
            state.finish_query_with_detail(&request, revision, phase, status, detail);
            let _ = applied.send(());
        }
    });
    Ok(wait_for_query_application(wait_for_application, &cancellation).await?)
}

async fn wait_for_query_application(
    wait_for_application: oneshot::Receiver<()>,
    cancellation: &sillage_daemon::SessionSearchCancellation,
) -> Result<QueryOutcome, String> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Ok(QueryOutcome::Cancelled),
        result = wait_for_application => result
            .map(|()| QueryOutcome::Applied)
            .map_err(|_| "The selected-source UI result callback was dropped before application".to_string()),
    }
}

fn show_query_error(
    ui: &UiWeak,
    state: &Arc<SessionSourceState>,
    request: &super::state::SourceQuery,
    message: String,
) {
    let ui = ui.clone();
    let state = Arc::clone(state);
    let request = request.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if !state.query_is_current(&request) {
            return;
        }
        if let Some(window) = ui.upgrade() {
            window.set_index_status("Selected-source search unavailable".into());
            window.set_status_kind("error".into());
            window.set_status_message(message.clone().into());
            window.set_notice(message.into());
        }
    });
}

pub(super) fn clear_selected_source_results(frontend: &Frontend, ui: &UiWeak, notice: String) {
    let rows = {
        let mut model = crate::application::lock(&frontend.model);
        model.passage_search_pending = false;
        model.accepted_passages.clear();
        model.accepted_paths.clear();
        model.passages_loaded = true;
        model.content_view_passages.clear();
        model.displayed = (0..model.accepted.len())
            .map(crate::application::DisplayedResult::Application)
            .collect();
        model
            .accepted
            .iter()
            .map(crate::application::search::result_row)
            .collect::<Vec<_>>()
    };
    let ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_results(slint::ModelRc::new(slint::VecModel::from(rows)));
            window.set_passage_view_open(false);
            window.set_passage_view_results(crate::application::empty_results());
            window.set_actions(crate::application::empty_actions());
            window.set_actions_open(false);
            window.set_index_status("Selected Markdown source retired".into());
            window.set_status_kind("ready".into());
            window.set_status_message(notice.clone().into());
            window.set_notice(notice.into());
        }
    });
}

pub(super) fn withdraw_passage(
    window: &crate::LauncherWindow,
    frontend: &Frontend,
    result_id: &str,
    message: &str,
) {
    let query = crate::application::lock(&frontend.model).query.clone();
    let generation = frontend.generation.load(Ordering::Acquire);
    let mut model = crate::application::lock(&frontend.model);
    model
        .accepted_passages
        .retain(|accepted| accepted.result_id != result_id);
    model.content_view_passages.clear();
    let (rows, displayed, _) =
        crate::application::search::build_result_rows(&model, &query, generation);
    model.displayed = displayed;
    drop(model);
    window.set_results(slint::ModelRc::new(slint::VecModel::from(rows)));
    window.set_passage_view_open(false);
    window.set_passage_view_results(crate::application::empty_results());
    window.set_actions(crate::application::empty_actions());
    window.set_actions_open(false);
    window.set_status_kind("error".into());
    window.set_status_message(message.into());
    window.set_notice(message.into());
}

#[cfg(test)]
mod tests {
    use super::super::controller::build_choice;
    use super::super::response;
    use super::super::state::{LifecycleTestStage, SessionSourceState, SourcePhase};
    use super::{
        QueryOutcome, run_query, run_query_worker, settle_query_failure, wait_for_query_application,
    };
    use sillage_daemon::{
        SessionOperationSettlement, SessionPreparationOwner, SessionSearchCancellation,
    };
    use std::error::Error;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::{env, io, process, time::Duration};
    use tokio::sync::{oneshot, watch};
    use tokio::time::timeout;

    static NEXT_QUERY_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    fn query_test_directory() -> io::Result<PathBuf> {
        let path = env::temp_dir().join(format!(
            "sillage-source-query-{}-{}",
            process::id(),
            NEXT_QUERY_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        Ok(path)
    }

    fn test_frontend(state: Arc<SessionSourceState>) -> crate::application::Frontend {
        let (generation_updates, _) = watch::channel(0_u64);
        crate::application::Frontend {
            source_choice: state,
            generation: AtomicU64::new(0),
            active_search: AtomicU64::new(0),
            generation_updates,
            interactive_search: tokio::sync::Mutex::new(()),
            model: Mutex::new(crate::application::FrontendModel {
                query: String::new(),
                passage_search_pending: false,
                accepted: Vec::new(),
                selected_file: None,
                catalog_ticks_until_refresh: crate::application::CATALOG_REFRESH_TICKS,
                accepted_passages: Vec::new(),
                accepted_paths: Vec::new(),
                passages_loaded: false,
                displayed: Vec::new(),
                result_filter: "all".to_string(),
                content_view_passages: Vec::new(),
            }),
        }
    }

    async fn prepared_query_fixture() -> Result<(Arc<SessionSourceState>, PathBuf), Box<dyn Error>>
    {
        let directory = query_test_directory()?;
        let selected_path = directory.join("selected.md");
        std::fs::write(&selected_path, "# Query fixture\nA small local document.\n")?;
        let state = Arc::new(SessionSourceState::default());
        state.set_selection(selected_path);
        state.set_consent(true);
        let (epoch, path, retention, persistence, cancellation, terminal_slot) =
            state.begin_prepare().map_err(std::io::Error::other)?;
        let choice = build_choice(path, retention, persistence).map_err(std::io::Error::other)?;
        let owner = Arc::new(SessionPreparationOwner::new());
        let runtime = owner
            .prepare_cancellable(choice.clone(), cancellation.clone())
            .await
            .map_err(|error| io::Error::other(format!("{error:#}")))?;
        if !state.finish_prepare(epoch, &cancellation, owner, Arc::new(runtime), choice) {
            return Err(io::Error::other("source preparation did not settle").into());
        }
        drop(terminal_slot);
        Ok((state, directory))
    }

    async fn stale_budget_query_failure(
        state: &Arc<SessionSourceState>,
        frontend: &Arc<crate::application::Frontend>,
        ui: &super::UiWeak,
        requests: &mut watch::Receiver<Option<super::super::state::SourceQuery>>,
        generation: u64,
        next_generation: u64,
    ) -> Result<String, Box<dyn Error>> {
        let (ready, readiness) = oneshot::channel();
        state
            .submit_query("x".repeat(2_000_001), generation, readiness)
            .map_err(std::io::Error::other)?;
        let request = requests
            .borrow_and_update()
            .clone()
            .ok_or_else(|| io::Error::other("source query was not submitted"))?;
        ready
            .send(())
            .map_err(|_| io::Error::other("query readiness receiver was dropped"))?;
        let reservation = match state
            .begin_query_operation(&request)
            .map_err(std::io::Error::other)?
        {
            Some(reservation) => reservation,
            None => return Err(io::Error::other("query admission was rejected").into()),
        };
        let terminal_slot = reservation;
        let result = run_query(state, frontend, ui, &request).await;
        state.query_changed(next_generation);
        let error = match result {
            Err(error) => {
                if error.is_routine_settlement() {
                    return Err(io::Error::other(
                        "real over-budget API failure was misclassified as routine",
                    )
                    .into());
                }
                let message = error.message().to_string();
                settle_query_failure(state, ui, &request, terminal_slot, error);
                message
            }
            Ok(_) => return Err(io::Error::other("query unexpectedly succeeded").into()),
        };
        state.finish_query_operation(&request);
        Ok(error)
    }

    #[test]
    fn typed_settlement_variants_are_routine() {
        let cancellation = SessionOperationSettlement::RoutineCancellation;
        let deadline = SessionOperationSettlement::DeadlineExceeded;
        let typed = response::SourceOperationFailure::from_operation_message(
            "original operation error".to_string(),
            Some(cancellation),
        );
        let timed_out = response::SourceOperationFailure::from_operation_message(
            "original deadline error".to_string(),
            Some(deadline),
        );
        assert!(typed.is_routine_settlement());
        assert!(timed_out.is_routine_settlement());
    }

    #[tokio::test]
    async fn query_worker_exits_when_started_after_shutdown_notification() -> Result<(), String> {
        let state = Arc::new(SessionSourceState::default());
        state.stop_new_admission();
        let frontend = Arc::new(test_frontend(Arc::clone(&state)));

        match timeout(
            Duration::from_millis(50),
            run_query_worker(state, frontend, slint::Weak::default()),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err("query worker waited for a new request after shutdown".to_string()),
        }
    }

    #[tokio::test]
    async fn stale_real_query_failures_are_all_retained_until_shutdown()
    -> Result<(), Box<dyn Error>> {
        let (state, directory) = prepared_query_fixture().await?;
        state.query_changed(1);
        let mut requests = state.subscribe_queries();
        let frontend = Arc::new(test_frontend(Arc::clone(&state)));
        let ui = slint::Weak::default();
        let first = stale_budget_query_failure(&state, &frontend, &ui, &mut requests, 1, 2).await?;
        let second =
            stale_budget_query_failure(&state, &frontend, &ui, &mut requests, 2, 3).await?;
        state.retire_active().await.map_err(std::io::Error::other)?;
        std::fs::remove_dir_all(directory)?;

        assert!(first.contains("session search request exceeds the selected source budget"));
        assert!(second.contains("session search request exceeds the selected source budget"));
        let failures = state.take_terminal_operation_failures();
        assert!(
            failures.len() == 2 && failures[0] == first && failures[1] == second,
            "stale query failures were not retained in order"
        );
        Ok(())
    }

    #[tokio::test]
    async fn query_error_is_retained_when_request_changes_before_atomic_settlement()
    -> Result<(), Box<dyn Error>> {
        let (state, directory) = prepared_query_fixture().await?;
        state.query_changed(1);
        let mut requests = state.subscribe_queries();
        let frontend = Arc::new(test_frontend(Arc::clone(&state)));
        let ui = slint::Weak::default();
        let (ready, readiness) = oneshot::channel();
        state
            .submit_query("x".repeat(2_000_001), 1, readiness)
            .map_err(io::Error::other)?;
        let request = requests
            .borrow_and_update()
            .clone()
            .ok_or_else(|| io::Error::other("source query was not submitted"))?;
        ready
            .send(())
            .map_err(|_| io::Error::other("query readiness receiver was dropped"))?;
        let reservation = state
            .begin_query_operation(&request)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("query admission was rejected"))?;
        let terminal_slot = reservation;
        let error = match run_query(&state, &frontend, &ui, &request).await {
            Err(error) if !error.is_routine_settlement() => error,
            Err(_) => {
                return Err(io::Error::other("real over-budget query failure was routine").into());
            }
            Ok(_) => {
                return Err(io::Error::other("over-budget query unexpectedly succeeded").into());
            }
        };
        let message = error.message().to_string();

        let (entered, wait_entered) = std::sync::mpsc::sync_channel(1);
        let (release, wait_release) = std::sync::mpsc::sync_channel(1);
        state.install_lifecycle_test_gate(
            LifecycleTestStage::QueryBeforeAtomicFailureSettlement,
            entered,
            wait_release,
        );
        let update_state = Arc::clone(&state);
        let gate_message = message.clone();
        let update = std::thread::spawn(move || -> io::Result<()> {
            let observation = wait_entered.recv().map_err(io::Error::other)?;
            if !observation.failed {
                return Err(io::Error::other("query failure gate received no failure"));
            }
            if observation.original_error.as_deref() != Some(gate_message.as_str()) {
                return Err(io::Error::other(
                    "query failure gate changed the original error",
                ));
            }
            update_state.query_changed(2);
            release
                .send(())
                .map_err(|_| io::Error::other("query failure gate disconnected"))?;
            Ok(())
        });
        settle_query_failure(&state, &ui, &request, terminal_slot, error);
        update
            .join()
            .map_err(|_| io::Error::other("query race updater panicked"))??;
        state.finish_query_operation(&request);
        state.retire_active().await.map_err(io::Error::other)?;
        let failures = state.take_terminal_operation_failures();
        std::fs::remove_dir_all(directory)?;

        assert!(message.contains("session search request exceeds the selected source budget"));
        assert!(
            failures.len() == 1 && failures.first().is_some_and(|failure| failure == &message),
            "stale query failure was not retained with its original error"
        );
        Ok(())
    }

    #[tokio::test]
    async fn source_lifecycle_rejects_query_during_retirement() -> Result<(), Box<dyn Error>> {
        let (state, directory) = prepared_query_fixture().await?;
        let mut requests = state.subscribe_queries();
        let (_held_ready, held_readiness) = oneshot::channel();
        state
            .submit_query("held query".to_string(), 0, held_readiness)
            .map_err(io::Error::other)?;
        let held_request = requests
            .borrow_and_update()
            .clone()
            .ok_or_else(|| io::Error::other("held query was not submitted"))?;
        let held_reservation = state
            .begin_query_operation(&held_request)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("held query operation was not admitted"))?;

        state.set_retiring();
        let retirement = state.retire_active();
        tokio::pin!(retirement);
        let retirement_state = timeout(Duration::from_secs(30), async {
            tokio::select! {
                _ = held_request.cancellation.cancelled() => None,
                result = &mut retirement => Some(result),
            }
        })
        .await;
        let (retirement_was_held, early_retirement) = match retirement_state {
            Ok(None) => (true, None),
            Ok(Some(result)) => (false, Some(result)),
            Err(_) => (false, None),
        };

        let (_query_ready, query_readiness) = oneshot::channel();
        let submit_result = state.submit_query(
            "query submitted during retirement".to_string(),
            0,
            query_readiness,
        );
        let phase_during_submit = state.snapshot().phase;
        let request_during_retirement = if submit_result.is_ok() {
            requests.borrow_and_update().clone()
        } else {
            None
        };
        let controls = state.snapshot();
        let control_change_rejected = if let Err(error) = state.set_controls(
            sillage_domain::DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
            controls.control_persistence,
        ) {
            state.set_error(error);
            true
        } else {
            false
        };
        let control_error_snapshot = state.snapshot();
        let phase_after_control_error = control_error_snapshot.phase;
        let status_after_control_error = control_error_snapshot.status;
        let message_after_control_error = control_error_snapshot.error;
        let (_, control_error_query_readiness) = oneshot::channel();
        let control_error_submit_result = state.submit_query(
            "query submitted after control-change error".to_string(),
            0,
            control_error_query_readiness,
        );
        let phase_after_control_error_submit = state.snapshot().phase;

        state.finish_query_operation(&held_request);
        drop(held_reservation);
        let operation_admitted = match request_during_retirement {
            Some(request) => match state.begin_query_operation(&request) {
                Ok(Some(reservation)) => {
                    state.finish_query_operation(&request);
                    drop(reservation);
                    Ok(true)
                }
                Ok(None) => Ok(false),
                Err(error) => Err(error),
            },
            None => Ok(false),
        };
        state.cancel_operations();
        let retirement_result = match early_retirement {
            Some(result) => result,
            None => retirement.await,
        };
        retirement_result.map_err(io::Error::other)?;
        let operation_admitted = operation_admitted.map_err(io::Error::other)?;
        let can_search_after_retirement = state.snapshot().can_search;
        std::fs::remove_dir_all(directory)?;

        assert!(
            retirement_was_held,
            "retirement was not held by owned query work"
        );
        assert!(
            submit_result.is_err(),
            "foreground query was accepted during source retirement"
        );
        assert_eq!(phase_during_submit, SourcePhase::Retiring);
        assert!(!operation_admitted, "query work started during retirement");
        assert!(!can_search_after_retirement);
        assert!(
            control_change_rejected,
            "retention change was not rejected while owned source work was retiring"
        );
        assert_eq!(
            phase_after_control_error,
            SourcePhase::Retiring,
            "control-change error cleared the source's retirement ownership phase"
        );
        assert_eq!(
            status_after_control_error,
            "Selected-source operation failed"
        );
        assert_eq!(
            message_after_control_error,
            "Retire or purge the prepared source before changing documentary retention"
        );
        assert!(
            control_error_submit_result.is_err(),
            "foreground query was accepted after a control-change error during retirement"
        );
        assert_eq!(phase_after_control_error_submit, SourcePhase::Retiring);
        Ok(())
    }
    #[tokio::test]
    async fn source_lifecycle_preserves_stop_during_rejected_worker_admission()
    -> Result<(), Box<dyn Error>> {
        let (state, directory) = prepared_query_fixture().await?;
        let mut requests = state.subscribe_queries();
        let (_query_ready, query_readiness) = oneshot::channel();
        let submitted =
            state.submit_query("query queued before stop".to_string(), 0, query_readiness);
        let request = if submitted.is_ok() {
            requests.borrow_and_update().clone()
        } else {
            None
        };
        let Some(request) = request else {
            let retirement = state.retire_active().await;
            if retirement.is_ok() {
                state.mark_stopped();
            }
            retirement.map_err(io::Error::other)?;
            std::fs::remove_dir_all(&directory)?;
            return Err(io::Error::other("query fixture request was not submitted").into());
        };

        // The failure vector is preallocated from the production reservation limit.
        let prior_failures = state.take_terminal_operation_failures();
        let prior_failures_empty = prior_failures.is_empty();
        let reservation_limit = prior_failures.capacity();
        drop(prior_failures);
        let mut terminal_reservations = Vec::with_capacity(reservation_limit);
        let mut reservation_limit_reached = false;
        for _ in 0..=reservation_limit {
            match state.reserve_terminal_operation() {
                Ok(reservation) => terminal_reservations.push(reservation),
                Err(_) => {
                    reservation_limit_reached = true;
                    break;
                }
            }
        }
        let reservations_held = terminal_reservations.len();

        let mut unexpected_admission = false;
        let worker_error = match state.begin_query_operation(&request) {
            Err(error) => Some(error),
            Ok(None) => None,
            Ok(Some(reservation)) => {
                unexpected_admission = true;
                drop(reservation);
                state.finish_query_operation(&request);
                None
            }
        };

        state.stop_new_admission();
        let phase_after_stop = state.snapshot().phase;
        let request_current_after_stop = state.query_request_is_current(&request);
        let worker_error_published = if let Some(error) = worker_error.as_ref() {
            if state.query_request_is_current(&request) {
                state.fail_query(&request, error.clone(), false);
                true
            } else {
                false
            }
        } else {
            false
        };
        let snapshot_after_worker_error = state.snapshot();
        let phase_after_worker_error = snapshot_after_worker_error.phase;
        let worker_error_visible = worker_error
            .as_ref()
            .is_some_and(|error| snapshot_after_worker_error.error.as_str() == error.as_str());
        let (_, future_readiness) = oneshot::channel();
        let future_submit_result = state.submit_query(
            "query submitted after stop".to_string(),
            0,
            future_readiness,
        );
        let phase_after_future_submit = state.snapshot().phase;

        drop(terminal_reservations);
        let retirement = state.retire_active().await;
        if retirement.is_ok() {
            state.mark_stopped();
        }
        let terminal_failures_empty = state.take_terminal_operation_failures().is_empty();
        retirement.map_err(io::Error::other)?;
        std::fs::remove_dir_all(&directory)?;

        assert!(prior_failures_empty);
        assert!(reservation_limit_reached);
        assert!(reservations_held > 0);
        assert!(!unexpected_admission);
        assert!(worker_error.is_some());
        assert_eq!(phase_after_stop, SourcePhase::Retiring);
        assert!(request_current_after_stop);
        assert!(worker_error_published);
        assert!(worker_error_visible);
        assert_eq!(phase_after_worker_error, SourcePhase::Retiring);
        assert!(future_submit_result.is_err());
        assert_eq!(phase_after_future_submit, SourcePhase::Retiring);
        assert!(terminal_failures_empty);
        Ok(())
    }

    #[tokio::test]
    async fn shutdown_cancellation_releases_unacknowledged_queued_ui_result() -> Result<(), String>
    {
        let (applied, wait_for_application) = oneshot::channel();
        let queued_callback = move || {
            let _ = applied.send(());
        };
        let cancellation = SessionSearchCancellation::new();
        let wait = wait_for_query_application(wait_for_application, &cancellation);
        tokio::pin!(wait);

        let remained_pending = tokio::select! {
            biased;
            _result = &mut wait => false,
            () = std::future::ready(()) => true,
        };
        assert!(
            remained_pending,
            "queued UI result completed before cancellation"
        );

        cancellation.cancel();
        let outcome = wait
            .await
            .map_err(|error| format!("query UI acknowledgement wait failed: {error}"))?;
        assert_eq!(outcome, QueryOutcome::Cancelled);
        drop(queued_callback);
        Ok(())
    }
}
