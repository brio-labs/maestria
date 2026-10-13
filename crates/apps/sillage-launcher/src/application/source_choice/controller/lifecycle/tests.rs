use super::super::build_choice;
use super::*;
use crate::application::source_choice::state::{LifecycleTestStage, SourcePhase};
use sillage_daemon::{SessionOperationSettlement, SessionPreparationOwner};
use std::error::Error;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{env, io, process, time::Duration};
use tokio::sync::watch;
use tokio::time::timeout;

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn test_directory() -> io::Result<PathBuf> {
    let path = env::temp_dir().join(format!(
        "sillage-source-lifecycle-{}-{}",
        process::id(),
        NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path)?;
    Ok(path)
}

fn test_frontend(state: Arc<SessionSourceState>) -> Arc<Frontend> {
    let (generation_updates, _) = watch::channel(0_u64);
    Arc::new(Frontend {
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
    })
}

async fn run_remove_metadata_transition(
    documentary: DocumentaryRetentionChoice,
    current_persistence: ControlPersistenceChoice,
    stage: LifecycleTestStage,
    next_persistence: ControlPersistenceChoice,
) -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let selected_path = directory.join("selected.md");
    std::fs::write(&selected_path, "# Selected source\n")?;
    let state = Arc::new(SessionSourceState::default());
    state
        .set_controls(documentary, current_persistence)
        .map_err(io::Error::other)?;
    state.set_selection(selected_path.clone());
    let launcher = Arc::new(LauncherState::new(SettingsManager::load(Ok(
        directory.clone()
    )))?);
    commit_selected_source_path(&state, &launcher, &selected_path).map_err(io::Error::other)?;

    let (entered, wait_entered) = std::sync::mpsc::sync_channel(1);
    let (release, wait_release) = std::sync::mpsc::sync_channel(1);
    state.install_lifecycle_test_gate(stage, entered, wait_release);
    let frontend = test_frontend(Arc::clone(&state));
    let folder_search_setup: Arc<Mutex<Option<Arc<SearchSetupController>>>> =
        Arc::new(Mutex::new(None));
    let ui = UiWeak::default();
    let remove_state = Arc::clone(&state);
    let remove_launcher = Arc::clone(&launcher);
    let remove_frontend = Arc::clone(&frontend);
    let removal = tokio::spawn(async move {
        remove_source(
            &remove_state,
            &remove_launcher,
            &remove_frontend,
            &ui,
            &folder_search_setup,
        )
        .await;
    });
    let observation = wait_entered.recv().map_err(io::Error::other)?;
    if !observation.failed {
        return Err(io::Error::other("remove test gate received no entry").into());
    }

    let serialized = state.control_metadata_transition_is_locked();
    let update_before = if serialized {
        None
    } else {
        Some(update_source_controls(
            &state,
            &launcher,
            documentary,
            next_persistence,
        ))
    };
    release
        .send(())
        .map_err(|_| io::Error::other("removal test gate disconnected"))?;
    removal.await.map_err(io::Error::other)?;
    let update_after = if serialized {
        Some(update_source_controls(
            &state,
            &launcher,
            documentary,
            next_persistence,
        ))
    } else {
        None
    };

    let update_result = match (update_before, update_after) {
        (Some(result), None) | (None, Some(result)) => result,
        _ => Err("source-control update did not run exactly once".to_string()),
    };
    let restored = SettingsManager::load(Ok(directory.clone())).source_control_preferences();
    let selected_path_after = state.selected_path();
    std::fs::remove_dir_all(directory)?;
    update_result.map_err(io::Error::other)?;

    let expected_documentary = if next_persistence == ControlPersistenceChoice::SessionOnly {
        DocumentaryRetentionChoice::NoDocumentaryPersistence
    } else {
        documentary
    };
    assert!(
        selected_path_after.is_none(),
        "removal did not clear the selected source path"
    );
    assert_eq!(restored.documentary_retention, expected_documentary);
    assert_eq!(restored.control_persistence, next_persistence);
    assert!(
        restored.selected_scope.is_none(),
        "removal did not clear the persisted selected scope"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_serializes_a_persist_off_after_its_snapshot() -> Result<(), Box<dyn Error>> {
    run_remove_metadata_transition(
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
        ControlPersistenceChoice::PersistExplicitly,
        LifecycleTestStage::RemoveAfterSnapshot,
        ControlPersistenceChoice::SessionOnly,
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_serializes_persist_on_before_selection_clear() -> Result<(), Box<dyn Error>> {
    run_remove_metadata_transition(
        DocumentaryRetentionChoice::NoDocumentaryPersistence,
        ControlPersistenceChoice::SessionOnly,
        LifecycleTestStage::RemoveAfterSettingsWrite,
        ControlPersistenceChoice::PersistExplicitly,
    )
    .await
}

#[test]
fn selected_path_is_persisted_before_preparation_without_documentary_retention()
-> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let selected_path = directory.join("selected.md");
    let state = SessionSourceState::default();
    state
        .set_controls(
            DocumentaryRetentionChoice::NoDocumentaryPersistence,
            ControlPersistenceChoice::PersistExplicitly,
        )
        .map_err(std::io::Error::other)?;
    let snapshot = state.snapshot();
    let mut settings = SettingsManager::load(Ok(directory.clone()));

    persist_selected_source_path(
        &mut settings,
        snapshot.documentary_retention,
        snapshot.control_persistence,
        &selected_path,
    )
    .map_err(std::io::Error::other)?;

    state.set_selection(selected_path.clone());
    let restored = SettingsManager::load(Ok(directory.clone())).source_control_preferences();
    assert_eq!(
        restored.selected_scope,
        Some(SourceScope::file(selected_path).map_err(std::io::Error::other)?)
    );
    assert_eq!(
        restored.documentary_retention,
        DocumentaryRetentionChoice::NoDocumentaryPersistence
    );
    assert_eq!(
        restored.control_persistence,
        ControlPersistenceChoice::PersistExplicitly
    );
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn stale_prepare_metadata_cannot_restore_persist_off_scope() -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let selected_path = directory.join("selected.md");
    std::fs::write(&selected_path, "# Selected source\n\nPrepare fixture.\n")?;
    let state = Arc::new(SessionSourceState::default());
    state
        .set_controls(
            DocumentaryRetentionChoice::NoDocumentaryPersistence,
            ControlPersistenceChoice::PersistExplicitly,
        )
        .map_err(std::io::Error::other)?;
    state.set_selection(selected_path.clone());
    let launcher = Arc::new(LauncherState::new(SettingsManager::load(Ok(
        directory.clone()
    )))?);
    commit_selected_source_path(&state, &launcher, &selected_path)
        .map_err(std::io::Error::other)?;
    state.set_consent(true);

    let (prepare_entered, entered) = oneshot::channel();
    let (release, release_prepare) = oneshot::channel();
    state.install_prepare_test_gate(prepare_entered, release_prepare);
    let (requests, request_receiver) = mpsc::channel(2);
    let worker = tokio::spawn(source_worker(
        Arc::clone(&state),
        Arc::clone(&launcher),
        test_frontend(Arc::clone(&state)),
        UiWeak::default(),
        Arc::new(Mutex::new(None)),
        request_receiver,
    ));
    requests
        .send(SourceRequest::Prepare)
        .await
        .map_err(|_| io::Error::other("source worker stopped before Prepare"))?;
    entered.await.map_err(std::io::Error::other)?;

    let controls_updated = update_source_controls(
        &state,
        &launcher,
        DocumentaryRetentionChoice::NoDocumentaryPersistence,
        ControlPersistenceChoice::SessionOnly,
    );
    let _ = release.send(());
    controls_updated.map_err(std::io::Error::other)?;

    let preparation_finished = timeout(Duration::from_secs(30), async {
        loop {
            let phase = state.snapshot().phase;
            if phase != SourcePhase::Preparing {
                return phase;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(std::io::Error::other)?;
    let (shutdown, shutdown_result) = oneshot::channel();
    requests
        .send(SourceRequest::Shutdown(shutdown))
        .await
        .map_err(|_| io::Error::other("source worker stopped before shutdown"))?;
    shutdown_result
        .await
        .map_err(std::io::Error::other)?
        .map_err(std::io::Error::other)?;
    worker
        .await
        .map_err(std::io::Error::other)?
        .map_err(std::io::Error::other)?;

    if !matches!(
        preparation_finished,
        SourcePhase::Ready | SourcePhase::Partial
    ) {
        return Err(io::Error::other(format!(
            "real selected-source Prepare did not succeed: {preparation_finished:?}"
        ))
        .into());
    }
    let restored = SettingsManager::load(Ok(directory.clone())).source_control_preferences();
    assert_eq!(restored.selected_scope, None);
    assert_eq!(
        restored.control_persistence,
        ControlPersistenceChoice::SessionOnly
    );
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
#[tokio::test]
async fn source_lifecycle_rejects_cancelled_prepare_publication() -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let selected_path = directory.join("selected.md");
    std::fs::write(
        &selected_path,
        "# Selected source\n\nCancellation before publication.\n",
    )?;
    let state = SessionSourceState::default();
    state.set_selection(selected_path);
    state.set_consent(true);
    let (epoch, selected_path, retention, persistence, cancellation, terminal_slot) =
        state.begin_prepare().map_err(io::Error::other)?;
    let choice = build_choice(selected_path, retention, persistence).map_err(io::Error::other)?;
    let owner = Arc::new(SessionPreparationOwner::new());
    let runtime = owner
        .prepare_cancellable(choice.clone(), cancellation.clone())
        .await
        .map_err(|error| io::Error::other(format!("{error:#}")))?;

    let cancellation_was_live_at_precheck = !cancellation.is_cancelled();
    let cancellation_accepted = state.cancel_prepare();
    let cancellation_was_set = cancellation.is_cancelled();
    let runtime = Arc::new(runtime);
    let published = state.finish_prepare(
        epoch,
        &cancellation,
        Arc::clone(&owner),
        Arc::clone(&runtime),
        choice,
    );

    let cleanup = if published {
        state.retire_active().await
    } else {
        let runtime_result = runtime.retire().await;
        runtime.wait_for_idle().await;
        let owner_result = owner.retire().await;
        owner.wait_for_idle().await;
        match (runtime_result, owner_result) {
            (Ok(()), Ok(())) => Ok(()),
            (runtime_result, owner_result) => Err(format!(
                "runtime settlement: {}; preparation-owner settlement: {}",
                runtime_result.map_or_else(|error| format!("{error:#}"), |_| "settled".to_string()),
                owner_result.map_or_else(|error| format!("{error:#}"), |_| "settled".to_string()),
            )),
        }
    };
    let cleanup_error = cleanup.err();
    let (settlement_message, routine, retain_critical_failure) = match cleanup_error.as_ref() {
        Some(error) => (
            format!("Preparation was cancelled; settlement failed: {error}"),
            false,
            true,
        ),
        None => (
            "Preparation cancelled; owned work was settled".to_string(),
            true,
            false,
        ),
    };
    state.settle_prepare_failure(
        epoch,
        &cancellation,
        settlement_message,
        routine,
        retain_critical_failure,
        terminal_slot,
    );
    let settled_phase = state.snapshot().phase;
    std::fs::remove_dir_all(directory)?;
    if let Some(error) = cleanup_error {
        return Err(io::Error::other(error).into());
    }

    assert!(cancellation_was_live_at_precheck);
    assert!(cancellation_accepted);
    assert!(cancellation_was_set);
    assert!(!published, "cancelled successful preparation was published");
    assert_eq!(settled_phase, SourcePhase::Cancelled);
    Ok(())
}

#[tokio::test]
async fn typed_routine_prepare_settlement_is_not_retained() -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let selected_path = directory.join("selected.md");
    std::fs::write(
        &selected_path,
        "# Selected source\n\nCancellation fixture.\n",
    )?;
    let state = SessionSourceState::default();
    state.set_selection(selected_path);
    state.set_consent(true);
    let (epoch, selected_path, retention, persistence, cancellation, terminal_slot) =
        state.begin_prepare().map_err(std::io::Error::other)?;
    let choice =
        build_choice(selected_path, retention, persistence).map_err(std::io::Error::other)?;
    let owner = Arc::new(SessionPreparationOwner::new());
    cancellation.cancel();
    let (original, routine) = match owner
        .prepare_cancellable(choice, cancellation.clone())
        .await
    {
        Err(error) => {
            let routine = SessionOperationSettlement::classify_error(&error).is_some();
            (format!("{error:#}"), routine)
        }
        Ok(runtime) => {
            runtime.retire().await.map_err(std::io::Error::other)?;
            owner.retire().await.map_err(std::io::Error::other)?;
            return Err(io::Error::other("cancelled preparation unexpectedly succeeded").into());
        }
    };
    if !routine {
        return Err(io::Error::other(format!(
            "real cancelled preparation was not typed as routine: {original}"
        ))
        .into());
    }

    settle_prepare_failure(
        &state,
        epoch,
        owner,
        cancellation,
        terminal_slot,
        original,
        routine,
    )
    .await;
    assert!(state.take_terminal_operation_failures().is_empty());
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn stale_real_preparation_error_survives_terminal_settlement() -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let missing_path = directory.join("missing.md");
    let state = SessionSourceState::default();
    state.set_selection(missing_path.clone());
    state.set_consent(true);
    let (epoch, selected_path, retention, persistence, cancellation, terminal_slot) =
        state.begin_prepare().map_err(std::io::Error::other)?;
    let choice =
        build_choice(selected_path, retention, persistence).map_err(std::io::Error::other)?;
    let owner = Arc::new(SessionPreparationOwner::new());
    let (original, routine) = match owner
        .prepare_cancellable(choice, cancellation.clone())
        .await
    {
        Err(error) => {
            let routine = SessionOperationSettlement::classify_error(&error).is_some();
            (format!("{error:#}"), routine)
        }
        Ok(_) => {
            return Err(
                io::Error::other("preparation unexpectedly accepted a missing file").into(),
            );
        }
    };

    state.revoke_consent();
    settle_prepare_failure(
        &state,
        epoch,
        owner,
        cancellation,
        terminal_slot,
        original.clone(),
        routine,
    )
    .await;

    let failures = state.take_terminal_operation_failures();
    assert!(
        failures.len() == 1 && failures.first().is_some_and(|failure| failure == &original),
        "preparation failure was not retained with its original error"
    );
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn preparation_error_is_retained_when_epoch_changes_before_publication()
-> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let missing_path = directory.join("missing.md");
    let state = Arc::new(SessionSourceState::default());
    state.set_selection(missing_path);
    state.set_consent(true);
    let (epoch, selected_path, retention, persistence, cancellation, terminal_slot) =
        state.begin_prepare().map_err(io::Error::other)?;
    let choice = build_choice(selected_path, retention, persistence).map_err(io::Error::other)?;
    let owner = Arc::new(SessionPreparationOwner::new());
    let (original, routine) = match owner
        .prepare_cancellable(choice, cancellation.clone())
        .await
    {
        Err(error) => {
            let routine = SessionOperationSettlement::classify_error(&error).is_some();
            (format!("{error:#}"), routine)
        }
        Ok(_) => {
            return Err(
                io::Error::other("preparation unexpectedly accepted a missing file").into(),
            );
        }
    };
    if routine {
        return Err(io::Error::other("real missing-file failure was routine").into());
    }

    let (entered, wait_entered) = std::sync::mpsc::sync_channel(1);
    let (release, wait_release) = std::sync::mpsc::sync_channel(1);
    state.install_lifecycle_test_gate(
        LifecycleTestStage::PrepareBeforeAtomicFailureSettlement,
        entered,
        wait_release,
    );
    let revoke_state = Arc::clone(&state);
    let original_for_gate = original.clone();
    let revoke = std::thread::spawn(move || -> io::Result<()> {
        let observation = wait_entered.recv().map_err(io::Error::other)?;
        if !observation.failed {
            return Err(io::Error::other(
                "preparation failure gate received no failure",
            ));
        }
        if observation.original_error.as_deref() != Some(original_for_gate.as_str()) {
            return Err(io::Error::other(
                "preparation failure gate changed the original error",
            ));
        }
        revoke_state.revoke_consent();
        release
            .send(())
            .map_err(|_| io::Error::other("preparation failure gate disconnected"))?;
        Ok(())
    });
    settle_prepare_failure(
        &state,
        epoch,
        owner,
        cancellation,
        terminal_slot,
        original.clone(),
        routine,
    )
    .await;
    revoke
        .join()
        .map_err(|_| io::Error::other("preparation race updater panicked"))??;
    let failures = state.take_terminal_operation_failures();
    std::fs::remove_dir_all(directory)?;

    assert!(
        failures.len() == 1 && failures.first().is_some_and(|failure| failure == &original),
        "preparation failure was not retained with its original error"
    );
    Ok(())
}
