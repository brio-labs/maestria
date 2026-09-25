use super::*;
use crate::model::HOST_PREFERENCES;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn block_on<F: Future>(future: F) -> Result<F::Output, std::io::Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    Ok(runtime.block_on(future))
}

fn test_state() -> Result<LauncherState, LauncherError> {
    LauncherState::new(SettingsManager::load(Err(
        "test preferences are session-only".to_string(),
    )))
}

#[test]
fn rejects_stale_search_generations_before_worker_work() -> TestResult {
    let state = test_state()?;
    let first = block_on(state.search("quit".to_string(), 1))?;
    assert!(first.is_ok());
    let stale = block_on(state.search("quit".to_string(), 1))?;
    assert!(stale.is_err());
    assert_eq!(
        stale.err().map(|error| error.code).as_deref(),
        Some("stale_result")
    );
    Ok(())
}

#[test]
fn validates_accepted_action_and_prevents_double_dispatch() -> TestResult {
    let state = test_state()?;
    let search = block_on(state.search("quit".to_string(), 1))?;
    assert!(search.is_ok());
    let accepted = state.begin_action(HOST_QUIT, "quit", 1);
    assert!(accepted.is_ok());
    let duplicate = state.begin_action(HOST_QUIT, "quit", 1);
    assert!(duplicate.is_err());
    assert_eq!(
        duplicate.err().map(|error| error.code).as_deref(),
        Some("invalid_request")
    );
    state.finish_action();
    let forged_action = state.begin_action(HOST_QUIT, "not-registered", 1);
    assert!(forged_action.is_err());
    Ok(())
}

#[test]
fn rejects_forged_result_ids_with_stale_result_error() -> TestResult {
    let state = test_state()?;
    let search = block_on(state.search("quit".to_string(), 1))?;
    assert!(search.is_ok());
    let forged = state.begin_action(HOST_PREFERENCES, "open", 1);
    assert!(forged.is_err());
    assert_eq!(
        forged.err().map(|error| error.code).as_deref(),
        Some("stale_result")
    );
    Ok(())
}

#[test]
fn rejects_overlong_queries_without_advancing_generation() -> TestResult {
    let state = test_state()?;
    let query = "x".repeat(MAX_QUERY_BYTES + 1);
    let result = block_on(state.search(query, 1))?;
    assert!(result.is_err());
    let accepted = block_on(state.search("quit".to_string(), 1))?;
    assert!(accepted.is_ok());
    Ok(())
}

#[test]
fn stale_search_does_not_revoke_preferences_scope() -> TestResult {
    let state = test_state()?;
    block_on(state.search("quit".to_string(), 1))??;
    state.enter_preferences()?;
    let stale = block_on(state.search("quit".to_string(), 1))?;
    assert_eq!(
        stale.err().map(|error| error.code).as_deref(),
        Some("stale_result")
    );
    assert!(state.preferences_active()?);
    Ok(())
}

#[test]
fn superseded_file_selection_cannot_dispatch_an_effect() -> TestResult {
    let state = test_state()?;
    state.request_activation()?;
    let path = std::path::PathBuf::from("/tmp/maestria-selected-file");
    state.install_selected_file(path.clone(), 1)?;
    state.request_activation()?;
    let mut dispatched = false;
    let stale = state.with_current_file(1, &path, || {
        dispatched = true;
        Ok(())
    });
    assert_eq!(
        stale.err().map(|error| error.code).as_deref(),
        Some("stale_result")
    );
    assert!(!dispatched);
    Ok(())
}
