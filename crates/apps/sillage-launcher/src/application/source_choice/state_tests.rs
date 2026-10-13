use std::path::PathBuf;

use sillage_domain::{ControlPersistenceChoice, DocumentaryRetentionChoice, SourceScope};

use super::{SessionSourceState, SourcePhase};

fn test_frontend(
    source_choice: std::sync::Arc<SessionSourceState>,
    generation: u64,
    query: &str,
) -> crate::application::Frontend {
    let (generation_updates, _) = tokio::sync::watch::channel(generation);
    crate::application::Frontend {
        source_choice,
        generation: std::sync::atomic::AtomicU64::new(generation),
        active_search: std::sync::atomic::AtomicU64::new(0),
        generation_updates,
        interactive_search: tokio::sync::Mutex::new(()),
        model: std::sync::Mutex::new(crate::application::FrontendModel {
            query: query.to_string(),
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

fn test_launcher_state()
-> Result<std::sync::Arc<crate::ipc::LauncherState>, Box<dyn std::error::Error>> {
    let settings = crate::settings::SettingsManager::load(Err(
        "test launcher settings are session-only".to_string(),
    ));
    Ok(std::sync::Arc::new(crate::ipc::LauncherState::new(
        settings,
    )?))
}

#[tokio::test]
async fn selected_source_typed_generation_is_accepted_for_native_passage_actions()
-> Result<(), Box<dyn std::error::Error>> {
    let launcher = test_launcher_state()?;
    assert_eq!(launcher.mark_ready()?, 1);
    let source = std::sync::Arc::new(SessionSourceState::default());
    source.set_selection(PathBuf::from("/home/test/selected.md"));
    let frontend = std::sync::Arc::new(test_frontend(
        std::sync::Arc::clone(&source),
        1,
        "selected source",
    ));

    let typed = crate::application::search::start_typed_search(
        std::sync::Arc::clone(&launcher),
        std::sync::Arc::clone(&frontend),
        tokio::runtime::Handle::current(),
        slint::Weak::default(),
        "selected source".to_string(),
    )
    .ok_or("typed search generation unexpectedly exhausted")?;

    assert!(typed.source_route);
    launcher.begin_passage_action(typed.generation)?;
    launcher.finish_action();
    Ok(())
}

#[tokio::test]
async fn catalog_refresh_keeps_selected_source_results_and_native_generation()
-> Result<(), Box<dyn std::error::Error>> {
    let launcher = test_launcher_state()?;
    assert_eq!(launcher.mark_ready()?, 1);
    let source = std::sync::Arc::new(SessionSourceState::default());
    source.set_selection(PathBuf::from("/home/test/selected.md"));
    let frontend = std::sync::Arc::new(test_frontend(
        std::sync::Arc::clone(&source),
        1,
        "selected source",
    ));
    {
        let mut model = crate::application::lock(&frontend.model);
        model.accepted.push(crate::model::SearchResult {
            id: "selected-source-result".to_string(),
            kind: crate::model::ResultKind::Application,
            title: "Selected source result".to_string(),
            subtitle: "Exact selected Markdown file".to_string(),
            icon: None,
            actions: Vec::new(),
        });
    }

    crate::application::search::start_search(
        std::sync::Arc::clone(&launcher),
        std::sync::Arc::clone(&frontend),
        tokio::runtime::Handle::current(),
        slint::Weak::default(),
        "folder refresh".to_string(),
        None,
    );
    while frontend
        .active_search
        .load(std::sync::atomic::Ordering::Acquire)
        != 0
    {
        tokio::task::yield_now().await;
    }

    assert_eq!(
        frontend
            .generation
            .load(std::sync::atomic::Ordering::Acquire),
        1
    );
    {
        let model = crate::application::lock(&frontend.model);
        assert_eq!(model.query, "selected source");
        assert_eq!(model.accepted.len(), 1);
        assert_eq!(model.accepted[0].id, "selected-source-result");
    }
    launcher.begin_passage_action(1)?;
    launcher.finish_action();
    Ok(())
}

#[test]
fn documentary_retention_and_control_persistence_are_independent() -> Result<(), String> {
    let state = SessionSourceState::default();
    for documentary in [
        DocumentaryRetentionChoice::NoDocumentaryPersistence,
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
    ] {
        for controls in [
            ControlPersistenceChoice::SessionOnly,
            ControlPersistenceChoice::PersistExplicitly,
        ] {
            state.set_controls(documentary, controls)?;
            let snapshot = state.snapshot();
            assert_eq!(snapshot.documentary_retention, documentary);
            assert_eq!(snapshot.control_persistence, controls);
        }
    }
    Ok(())
}

#[test]
fn restored_source_controls_do_not_authorize_or_prepare_the_remembered_file() -> Result<(), String>
{
    let state = SessionSourceState::default();
    let path = PathBuf::from("/home/test/selected.md");
    let scope = SourceScope::file(path.clone()).map_err(|error| error.to_string())?;

    state.restore_controls(
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
        ControlPersistenceChoice::PersistExplicitly,
        Some(scope),
    );

    let snapshot = state.snapshot();
    assert_eq!(snapshot.selected_path, Some(path));
    assert_eq!(snapshot.phase, SourcePhase::Selected);
    assert!(!snapshot.consent);
    assert!(!snapshot.can_prepare);
    Ok(())
}

#[test]
fn explicitly_persisted_source_path_restores_without_documentary_retention() -> Result<(), String> {
    let state = SessionSourceState::default();
    let path = PathBuf::from("/home/test/selected.md");
    let scope = SourceScope::file(path.clone()).map_err(|error| error.to_string())?;

    state.restore_controls(
        DocumentaryRetentionChoice::NoDocumentaryPersistence,
        ControlPersistenceChoice::PersistExplicitly,
        Some(scope),
    );

    let snapshot = state.snapshot();
    assert_eq!(snapshot.selected_path, Some(path));
    assert_eq!(snapshot.phase, SourcePhase::Selected);
    assert!(!snapshot.consent);
    assert!(!snapshot.can_prepare);
    Ok(())
}

#[test]
fn cancelling_preparation_cancels_the_owned_token_and_records_cancellation() -> Result<(), String> {
    let state = SessionSourceState::default();
    state.set_selection(PathBuf::from("/home/test/selected.md"));
    state.set_consent(true);
    let (epoch, _, _, _, cancellation, terminal_slot) = state.begin_prepare()?;

    assert!(state.cancel_prepare());
    assert!(cancellation.is_cancelled());
    state.settle_prepare_failure(
        epoch,
        &cancellation,
        "owned preparation settled".to_string(),
        true,
        false,
        terminal_slot,
    );
    assert_eq!(state.snapshot().phase, SourcePhase::Cancelled);
    Ok(())
}

#[test]
fn disabling_control_persistence_during_preparation_does_not_cancel_owned_work()
-> Result<(), String> {
    let state = SessionSourceState::default();
    state.set_selection(PathBuf::from("/home/test/selected.md"));
    state.set_consent(true);
    state.set_controls(
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
        ControlPersistenceChoice::PersistExplicitly,
    )?;
    let (_, _, _, _, cancellation, _terminal_slot) = state.begin_prepare()?;

    state.set_controls(
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
        ControlPersistenceChoice::SessionOnly,
    )?;

    let snapshot = state.snapshot();
    assert_eq!(snapshot.phase, SourcePhase::Preparing);
    assert_eq!(
        snapshot.documentary_retention,
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged
    );
    assert_eq!(
        snapshot.control_persistence,
        ControlPersistenceChoice::SessionOnly
    );
    assert!(!cancellation.is_cancelled());
    Ok(())
}

#[test]
fn disabling_control_persistence_while_retiring_does_not_block_retirement() -> Result<(), String> {
    let state = SessionSourceState::default();
    state.set_selection(PathBuf::from("/home/test/selected.md"));
    state.set_controls(
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
        ControlPersistenceChoice::PersistExplicitly,
    )?;
    state.set_retiring();

    state.set_controls(
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
        ControlPersistenceChoice::SessionOnly,
    )?;

    let snapshot = state.snapshot();
    assert_eq!(snapshot.phase, SourcePhase::Retiring);
    assert_eq!(
        snapshot.documentary_retention,
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged
    );
    assert_eq!(
        snapshot.control_persistence,
        ControlPersistenceChoice::SessionOnly
    );
    Ok(())
}

#[test]
fn retiring_transition_keeps_selected_source_route_exclusive_until_removal() {
    let state = SessionSourceState::default();
    assert!(!state.is_selected());

    state.set_retiring();
    assert!(state.is_selected());

    state.clear_selection();
    assert!(!state.is_selected());
}
