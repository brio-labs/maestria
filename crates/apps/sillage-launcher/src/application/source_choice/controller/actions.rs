use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::mpsc;

use super::super::response::{SourceOperationFailure, reopen_current};
#[cfg(test)]
use super::super::state::LifecycleTestStage;
use super::super::state::{SessionSourceState, TerminalOperationReservation};
use crate::application::{AcceptedPassage, Frontend, UiWeak};
use crate::ipc::LauncherState;

pub(super) enum ProofRequest {
    OpenView {
        accepted: AcceptedPassage,
        generation: u64,
    },
    Action {
        accepted: AcceptedPassage,
        generation: u64,
        action_id: String,
    },
}

pub(super) async fn proof_worker(
    source: Arc<SessionSourceState>,
    frontend: Arc<Frontend>,
    launcher_state: Arc<LauncherState>,
    ui: UiWeak,
    mut requests: mpsc::Receiver<ProofRequest>,
) -> Result<(), String> {
    while let Some(request) = requests.recv().await {
        match request {
            ProofRequest::OpenView {
                accepted,
                generation,
            } => {
                open_current_evidence(&source, &frontend, &ui, accepted, generation).await;
            }
            ProofRequest::Action {
                accepted,
                generation,
                action_id,
            } => {
                apply_current_action(
                    &source,
                    &frontend,
                    &launcher_state,
                    &ui,
                    accepted,
                    generation,
                    action_id,
                )
                .await;
            }
        }
    }
    Ok(())
}

async fn open_current_evidence(
    source: &Arc<SessionSourceState>,
    frontend: &Arc<Frontend>,
    ui: &UiWeak,
    accepted: AcceptedPassage,
    generation: u64,
) {
    let Some(binding) = accepted.passage.session_binding.clone() else {
        return;
    };
    let lease = match source.begin_proof(&binding) {
        Ok(lease) => lease,
        Err(error) => {
            withdraw(ui, frontend, &accepted.result_id, error);
            return;
        }
    };
    let result = reopen_current(&lease.runtime, &binding, &lease.cancellation).await;
    retain_proof_failure(lease.terminal_slot, result.as_ref().err());
    source.finish_proof();

    #[cfg(test)]
    source.wait_at_lifecycle_test_gate(
        LifecycleTestStage::ProofOpenViewAfterReopen,
        result.is_err(),
        result
            .as_ref()
            .err()
            .map(|error| error.message().to_string()),
    );
    let current = source.binding_is_current(&binding)
        && frontend.generation.load(Ordering::Acquire) == generation
        && crate::application::search::passage_result_is_visible(frontend, &accepted.result_id);
    let weak = ui.clone();
    let frontend = Arc::clone(frontend);
    let source = Arc::clone(source);
    let _ = slint::invoke_from_event_loop(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if !current || !source.binding_is_current(&binding) {
            withdraw_in_window(
                &window,
                &frontend,
                &accepted.result_id,
                "This evidence belongs to an older selected-source search",
            );
            return;
        }
        match result {
            Ok(reopened) => crate::application::search::show_reopened_passage(
                &window,
                &frontend,
                &accepted,
                &reopened.citation,
                &reopened.excerpt,
            ),
            Err(error) => {
                withdraw_in_window(&window, &frontend, &accepted.result_id, error.message())
            }
        }
    });
}

async fn apply_current_action(
    source: &Arc<SessionSourceState>,
    frontend: &Arc<Frontend>,
    launcher_state: &Arc<LauncherState>,
    ui: &UiWeak,
    accepted: AcceptedPassage,
    generation: u64,
    action_id: String,
) {
    let Some(binding) = accepted.passage.session_binding.clone() else {
        launcher_state.finish_action();
        return;
    };
    let lease = match source.begin_proof(&binding) {
        Ok(lease) => lease,
        Err(error) => {
            launcher_state.finish_action();
            withdraw(ui, frontend, &accepted.result_id, error);
            return;
        }
    };
    let reopened = reopen_current(&lease.runtime, &binding, &lease.cancellation).await;
    retain_proof_failure(lease.terminal_slot, reopened.as_ref().err());
    source.finish_proof();

    #[cfg(test)]
    source.wait_at_lifecycle_test_gate(
        LifecycleTestStage::ProofActionAfterReopen,
        reopened.is_err(),
        reopened
            .as_ref()
            .err()
            .map(|error| error.message().to_string()),
    );
    let weak = ui.clone();
    let frontend = Arc::clone(frontend);
    let source = Arc::clone(source);
    let launcher_state = Arc::clone(launcher_state);
    let _ = slint::invoke_from_event_loop(move || {
        if frontend.generation.load(Ordering::Acquire) != generation
            || !source.binding_is_current(&binding)
        {
            launcher_state.finish_action();
            withdraw(
                &weak,
                &frontend,
                &accepted.result_id,
                "This evidence belongs to an older selected-source search".to_string(),
            );
            return;
        }
        let reopened = match reopened {
            Ok(reopened) => reopened,
            Err(error) => {
                launcher_state.finish_action();
                withdraw(
                    &weak,
                    &frontend,
                    &accepted.result_id,
                    error.message().to_string(),
                );
                return;
            }
        };
        let prepared =
            crate::application::search::passage_actions::action_from_reopened(reopened, &action_id);
        crate::application::search::passage_actions::finish_passage_action(
            launcher_state,
            frontend,
            weak,
            generation,
            accepted,
            prepared,
        );
    });
}

fn retain_proof_failure(
    reservation: TerminalOperationReservation<'_>,
    failure: Option<&SourceOperationFailure>,
) {
    if let Some(failure) = failure
        && !failure.is_routine_settlement()
    {
        reservation.record(failure.message().to_string());
    }
}

fn withdraw(ui: &UiWeak, frontend: &Arc<Frontend>, result_id: &str, message: String) {
    let Some(window) = ui.upgrade() else {
        return;
    };
    withdraw_in_window(&window, frontend, result_id, &message);
}

fn withdraw_in_window(
    window: &crate::LauncherWindow,
    frontend: &Frontend,
    result_id: &str,
    message: &str,
) {
    crate::application::source_choice::search::withdraw_passage(
        window, frontend, result_id, message,
    );
}

#[cfg(test)]
mod tests {
    use super::{apply_current_action, open_current_evidence, reopen_current};
    use crate::application::source_choice::state::{LifecycleTestStage, SessionSourceState};
    use crate::application::{AcceptedPassage, DisplayedResult, Frontend, FrontendModel};
    use crate::ipc::LauncherState;
    use crate::settings::SettingsManager;
    use sillage_daemon::SessionPreparationOwner;
    use std::error::Error;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::{env, io, process};
    use tokio::sync::{oneshot, watch};

    static NEXT_PROOF_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct ProofFixture {
        state: Arc<SessionSourceState>,
        frontend: Arc<Frontend>,
        launcher_state: Arc<LauncherState>,
        accepted: AcceptedPassage,
        directory: PathBuf,
        selected_path: PathBuf,
        mapped_unknown_candidate: bool,
    }

    fn test_directory() -> io::Result<PathBuf> {
        let path = env::temp_dir().join(format!(
            "sillage-source-proof-{}-{}",
            process::id(),
            NEXT_PROOF_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        Ok(path)
    }

    fn test_frontend(state: Arc<SessionSourceState>) -> Arc<Frontend> {
        let (generation_updates, _) = watch::channel(1_u64);
        Arc::new(Frontend {
            source_choice: state,
            generation: AtomicU64::new(1),
            active_search: AtomicU64::new(1),
            generation_updates,
            interactive_search: tokio::sync::Mutex::new(()),
            model: Mutex::new(FrontendModel {
                query: "saffron comet".to_string(),
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

    async fn proof_fixture() -> Result<ProofFixture, Box<dyn Error>> {
        let directory = test_directory()?;
        let selected_path = directory.join("selected.md");
        std::fs::write(
            &selected_path,
            "# Field notes\n\nThe saffron comet crosses the observatory at midnight.\n",
        )?;
        let state = Arc::new(SessionSourceState::default());
        state.set_selection(selected_path.clone());
        state.set_consent(true);
        let (epoch, path, retention, persistence, cancellation, terminal_slot) =
            state.begin_prepare().map_err(io::Error::other)?;
        let choice =
            super::super::build_choice(path, retention, persistence).map_err(io::Error::other)?;
        let owner = Arc::new(SessionPreparationOwner::new());
        let runtime = owner
            .prepare_cancellable(choice.clone(), cancellation.clone())
            .await
            .map_err(|error| io::Error::other(format!("{error:#}")))?;
        if !state.finish_prepare(epoch, &cancellation, owner, Arc::new(runtime), choice) {
            return Err(io::Error::other("source preparation did not settle").into());
        }
        drop(terminal_slot);

        state.query_changed(1);
        let mut requests = state.subscribe_queries();
        let (ready, readiness) = oneshot::channel();
        state
            .submit_query("saffron comet".to_string(), 1, readiness)
            .map_err(io::Error::other)?;
        let request = requests
            .borrow_and_update()
            .clone()
            .ok_or_else(|| io::Error::other("proof fixture query was not submitted"))?;
        ready
            .send(())
            .map_err(|_| io::Error::other("proof fixture query readiness was dropped"))?;
        let (runtime, source_path, _) = state
            .current_runtime_for_query(&request)
            .ok_or_else(|| io::Error::other("prepared runtime was not current"))?;
        let search_limit = runtime.choice().budget().max_results() as usize;
        let response = runtime
            .search_cancellable(
                request.query.clone(),
                search_limit,
                request.cancellation.clone(),
            )
            .await
            .map_err(|error| io::Error::other(format!("{error:#}")))?;
        let unknown_evidence_ids: Vec<u64> = response
            .outcome
            .evidence
            .iter()
            .filter(|candidate| candidate.freshness() == sillage_domain::FreshnessStatus::Unknown)
            .map(|candidate| candidate.evidence_id().value())
            .collect();
        let mapped = super::super::super::response::map_response(
            &runtime,
            &source_path,
            &request,
            response,
            search_limit,
        )
        .await
        .map_err(io::Error::other)?;
        let super::super::super::response::MappedSearch {
            mut passages,
            phase,
            status,
            detail,
            source_revision,
        } = mapped;
        let unknown_passage_index = passages
            .passages
            .iter()
            .position(|passage| unknown_evidence_ids.contains(&passage.evidence_id));
        state.finish_query_with_detail(&request, source_revision, phase, status, detail);
        let (passage, mapped_unknown_candidate) = if let Some(index) = unknown_passage_index {
            (passages.passages.remove(index), true)
        } else {
            let Some(passage) = passages.passages.into_iter().next() else {
                state.retire_active().await.map_err(io::Error::other)?;
                std::fs::remove_dir_all(&directory)?;
                return Err(
                    io::Error::other("real proof fixture query returned no evidence").into(),
                );
            };
            (passage, false)
        };
        let accepted = AcceptedPassage {
            result_id: format!("passage:{}:{}", request.generation, passage.evidence_id),
            passage,
        };
        let frontend = test_frontend(Arc::clone(&state));
        {
            let mut model = crate::application::lock(&frontend.model);
            model.accepted_passages.push(accepted.clone());
            model.displayed.push(DisplayedResult::Passage(0));
            model.passages_loaded = true;
        }
        let launcher_state = Arc::new(LauncherState::new(SettingsManager::load(Err(
            "proof lifecycle tests use session-only settings".to_string(),
        )))?);

        Ok(ProofFixture {
            state,
            frontend,
            launcher_state,
            accepted,
            directory,
            selected_path,
            mapped_unknown_candidate,
        })
    }

    #[tokio::test]
    async fn real_unknown_freshness_maps_to_authorized_passage_and_reopens_current_excerpt()
    -> Result<(), Box<dyn Error>> {
        let fixture = proof_fixture().await?;
        let mapped_unknown = fixture.mapped_unknown_candidate;
        let reopen_result = match fixture.accepted.passage.session_binding.as_ref() {
            Some(binding) => match fixture.state.begin_proof(binding) {
                Ok(lease) => {
                    let reopened = reopen_current(&lease.runtime, binding, &lease.cancellation)
                        .await
                        .map_err(|error| error.message().to_string());
                    fixture.state.finish_proof();
                    reopened
                }
                Err(error) => Err(error),
            },
            None => Err("mapped passage has no current-session evidence binding".to_string()),
        };
        let retirement = fixture.state.retire_active().await;
        let directory_cleanup = std::fs::remove_dir_all(&fixture.directory);
        retirement.map_err(io::Error::other)?;
        directory_cleanup?;
        let reopened = reopen_result.map_err(io::Error::other)?;

        assert!(
            mapped_unknown,
            "real selected-source result did not map an Unknown freshness candidate"
        );
        assert!(
            reopened.excerpt.contains("saffron comet"),
            "reopened current excerpt did not contain the real selected Markdown query"
        );
        assert!(
            reopened.path.as_ref() == Some(&fixture.selected_path),
            "current evidence reopened outside the selected source"
        );
        Ok(())
    }

    async fn run_stale_proof_failure(stage: LifecycleTestStage) -> Result<(), Box<dyn Error>> {
        let fixture = proof_fixture().await?;
        let replacement_path = fixture.directory.join("replacement.md");
        std::fs::write(
            &replacement_path,
            "# Replaced proof source\nThe original selected source has changed completely.\n",
        )?;
        std::fs::rename(&replacement_path, &fixture.selected_path)?;

        let (entered, wait_entered) = std::sync::mpsc::sync_channel(1);
        let (release, wait_release) = std::sync::mpsc::sync_channel(1);
        fixture
            .state
            .install_lifecycle_test_gate(stage, entered, wait_release);
        let update_state = Arc::clone(&fixture.state);
        let update_frontend = Arc::clone(&fixture.frontend);
        let update = std::thread::spawn(move || -> io::Result<String> {
            let observation = wait_entered.recv().map_err(io::Error::other)?;
            if !observation.failed {
                return Err(io::Error::other(
                    "proof error gate received no real failure",
                ));
            }
            let original_error = observation
                .original_error
                .filter(|error| !error.is_empty())
                .ok_or_else(|| io::Error::other("proof error gate omitted the original failure"))?;
            update_state.query_changed(2);
            update_frontend.generation.store(2, Ordering::Release);
            release
                .send(())
                .map_err(|_| io::Error::other("proof failure gate disconnected"))?;
            Ok(original_error)
        });
        let ui = slint::Weak::default();
        match stage {
            LifecycleTestStage::ProofOpenViewAfterReopen => {
                open_current_evidence(
                    &fixture.state,
                    &fixture.frontend,
                    &ui,
                    fixture.accepted.clone(),
                    1,
                )
                .await;
            }
            LifecycleTestStage::ProofActionAfterReopen => {
                apply_current_action(
                    &fixture.state,
                    &fixture.frontend,
                    &fixture.launcher_state,
                    &ui,
                    fixture.accepted.clone(),
                    1,
                    "open".to_string(),
                )
                .await;
            }
            _ => return Err(io::Error::other("invalid proof race stage").into()),
        }
        let original_error = update
            .join()
            .map_err(|_| io::Error::other("proof race updater panicked"))??;
        let failures = fixture.state.take_terminal_operation_failures();
        fixture
            .state
            .retire_active()
            .await
            .map_err(io::Error::other)?;
        std::fs::remove_dir_all(fixture.directory)?;

        assert!(
            failures.len() == 1
                && failures
                    .first()
                    .is_some_and(|failure| failure == &original_error),
            "proof failure was not retained exactly once with its original error"
        );
        Ok(())
    }

    #[tokio::test]
    async fn stale_open_view_error_is_retained_for_shutdown() -> Result<(), Box<dyn Error>> {
        run_stale_proof_failure(LifecycleTestStage::ProofOpenViewAfterReopen).await
    }

    #[tokio::test]
    async fn stale_action_error_is_retained_for_shutdown() -> Result<(), Box<dyn Error>> {
        run_stale_proof_failure(LifecycleTestStage::ProofActionAfterReopen).await
    }
}
