use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use sillage_domain::{
    ComputationTiming, ControlPersistenceChoice, DocumentaryRetentionChoice, SourceAvailability,
    SourceChoice, SourceChoiceParameters, SourceLocality, SourceRepresentation,
    SourceResourceBudget, SourceScope,
};
use slint::ComponentHandle;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::super::search_setup::SearchSetupController;
use super::state::SessionSourceState;

use crate::application::{AcceptedPassage, Frontend, LauncherWindow, UiWeak, lock};
use crate::ipc::LauncherState;

mod actions;
mod lifecycle;
mod presentation;
mod shutdown;
mod workers;

use self::actions::ProofRequest;
use self::lifecycle::SourceRequest;

pub(in crate::application) struct SourceChoiceController {
    state: Arc<SessionSourceState>,
    launcher_state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: Handle,
    ui: UiWeak,
    requests: mpsc::Sender<SourceRequest>,
    proof_requests: Mutex<Option<mpsc::Sender<ProofRequest>>>,
    stopping: AtomicBool,
    retire_enqueued: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<Result<(), String>>>>,
    folder_search_setup: Arc<Mutex<Option<Arc<SearchSetupController>>>>,
}

impl SourceChoiceController {
    pub(in crate::application) fn install(
        ui: &LauncherWindow,
        launcher_state: Arc<LauncherState>,
        frontend: Arc<Frontend>,
        runtime: Handle,
    ) -> Arc<Self> {
        let state = Arc::clone(&frontend.source_choice);
        if let Ok(settings) = launcher_state.settings() {
            let controls = settings.source_control_preferences();
            state.restore_controls(
                controls.documentary_retention,
                controls.control_persistence,
                controls.selected_scope,
            );
        }
        let (requests, request_receiver) = mpsc::channel(4);
        let (proof_requests, proof_receiver) = mpsc::channel(2);
        let controller = Arc::new(Self {
            state,
            launcher_state,
            frontend,
            runtime: runtime.clone(),
            ui: ui.as_weak(),
            requests,
            proof_requests: Mutex::new(Some(proof_requests)),
            stopping: AtomicBool::new(false),
            retire_enqueued: Arc::new(AtomicBool::new(false)),
            folder_search_setup: Arc::new(Mutex::new(None)),

            workers: Mutex::new(Vec::new()),
        });
        Self::install_callbacks(ui, &controller);
        controller.publish();
        controller.spawn_workers(request_receiver, proof_receiver);
        controller
    }
    pub(in crate::application) fn attach_search_setup(
        &self,
        search_setup: Arc<SearchSetupController>,
    ) {
        *lock(&self.folder_search_setup) = Some(search_setup);
    }

    fn install_callbacks(ui: &LauncherWindow, controller: &Arc<Self>) {
        let chooser = Arc::clone(controller);
        ui.on_source_file_requested(move || chooser.choose_source());
        let preparer = Arc::clone(controller);
        ui.on_source_prepare_requested(move || preparer.enqueue(SourceRequest::Prepare));
        let cancel = Arc::clone(controller);
        ui.on_source_cancel_requested(move || {
            if !cancel.state.cancel_prepare() {
                cancel.notice("There is no preparation to cancel.".to_string());
            }
        });
        let purge = Arc::clone(controller);
        ui.on_source_purge_requested(move || purge.enqueue(SourceRequest::Purge));
        let remove = Arc::clone(controller);
        ui.on_source_remove_requested(move || remove.enqueue(SourceRequest::Remove));
        let consent = Arc::clone(controller);
        ui.on_source_read_consent_changed(move |value| {
            if value {
                consent.state.set_consent(true);
            } else {
                consent.state.revoke_consent();
                consent.request_retire();
            }
        });
        let retention = Arc::clone(controller);
        ui.on_source_documentary_retention_changed(move |retain| {
            let documentary = if retain {
                DocumentaryRetentionChoice::RetainUntilExplicitlyPurged
            } else {
                DocumentaryRetentionChoice::NoDocumentaryPersistence
            };
            retention.change_controls(documentary, retention.state.snapshot().control_persistence);
        });
        let persistence = Arc::clone(controller);
        ui.on_source_control_persistence_changed(move |persist| {
            let control = if persist {
                ControlPersistenceChoice::PersistExplicitly
            } else {
                ControlPersistenceChoice::SessionOnly
            };
            persistence
                .change_controls(persistence.state.snapshot().documentary_retention, control);
        });
    }

    fn choose_source(self: &Arc<Self>) {
        if self.stopping.load(Ordering::Acquire) || !self.state.begin_picker() {
            return;
        }
        if let Err(error) = self.launcher_state.begin_modal() {
            self.state.finish_picker();
            self.state.set_error(error.message);
            return;
        }
        #[cfg(target_os = "linux")]
        let parent = self
            .ui
            .upgrade()
            .and_then(|window| crate::platform::window_identifier(window.window(), &self.runtime));
        #[cfg(not(target_os = "linux"))]
        let parent = None;
        if self
            .requests
            .try_send(SourceRequest::Choose(parent))
            .is_err()
        {
            self.launcher_state.end_modal();
            self.state.finish_picker();
            self.notice(
                "The selected-source operation queue is full or shutting down.".to_string(),
            );
        }
    }
    fn request_retire(&self) {
        if self.retire_enqueued.swap(true, Ordering::AcqRel) {
            return;
        }
        let sender = self.requests.clone();
        let enqueued = Arc::clone(&self.retire_enqueued);
        self.runtime.spawn(async move {
            if sender
                .send(SourceRequest::Retire(Arc::clone(&enqueued)))
                .await
                .is_err()
            {
                enqueued.store(false, Ordering::Release);
            }
        });
    }

    fn enqueue(&self, request: SourceRequest) {
        if self.stopping.load(Ordering::Acquire) || self.requests.try_send(request).is_err() {
            self.notice(
                "The selected-source operation queue is full or shutting down.".to_string(),
            );
        }
    }

    fn change_controls(
        &self,
        documentary_retention: DocumentaryRetentionChoice,
        control_persistence: ControlPersistenceChoice,
    ) {
        if let Err(error) = lifecycle::update_source_controls(
            &self.state,
            &self.launcher_state,
            documentary_retention,
            control_persistence,
        ) {
            self.state.set_error(error);
        }
    }

    pub(in crate::application) fn request_open_view(&self, result_id: &str) -> bool {
        let Some(accepted) = self.visible_session_passage(result_id) else {
            return false;
        };
        self.try_proof(ProofRequest::OpenView {
            accepted,
            generation: self.frontend.generation.load(Ordering::Acquire),
        });
        true
    }

    pub(in crate::application) fn request_action(
        &self,
        result_id: &str,
        action_id: String,
    ) -> bool {
        let Some(accepted) = self.visible_session_passage(result_id) else {
            return false;
        };
        let generation = self.frontend.generation.load(Ordering::Acquire);
        if let Err(error) = self.launcher_state.begin_passage_action(generation) {
            self.notice(error.message);
            return true;
        }
        if !self.try_proof(ProofRequest::Action {
            accepted,
            generation,
            action_id,
        }) {
            self.launcher_state.finish_action();
        }
        true
    }

    fn visible_session_passage(&self, result_id: &str) -> Option<AcceptedPassage> {
        if !crate::application::search::passage_result_is_visible(&self.frontend, result_id) {
            return None;
        }
        lock(&self.frontend.model)
            .accepted_passages
            .iter()
            .find(|accepted| {
                accepted.result_id == result_id && accepted.passage.session_binding.is_some()
            })
            .cloned()
    }

    fn try_proof(&self, request: ProofRequest) -> bool {
        let sender = lock(&self.proof_requests).as_ref().cloned();
        if sender.is_none_or(|sender| sender.try_send(request).is_err()) {
            self.notice(
                "A current-evidence operation is busy; retry after it settles.".to_string(),
            );
            false
        } else {
            true
        }
    }

    fn notice(&self, message: String) {
        let ui = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = ui.upgrade() {
                window.set_notice(message.into());
            }
        });
    }
}

pub(super) fn retained_root(path: &Path) -> Result<PathBuf, String> {
    let config = super::super::platform::config_dir().map_err(|error| error.to_string())?;
    let path_text = path
        .to_str()
        .ok_or_else(|| "The selected Markdown path is not valid UTF-8".to_string())?;
    use sha2::Digest;
    let digest = sha2::Sha256::digest(path_text.as_bytes());
    Ok(config
        .join("selected-source-retention")
        .join(format!("{digest:x}")))
}

pub(super) fn build_choice(
    path: PathBuf,
    documentary_retention: DocumentaryRetentionChoice,
    control_persistence: ControlPersistenceChoice,
) -> Result<SourceChoice, String> {
    let scope = SourceScope::file(path).map_err(|error| error.to_string())?;
    SourceChoice::try_new(SourceChoiceParameters {
        scope,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention,
        control_persistence,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::LexicalIndex],
        remote_disclosures: Vec::new(),
        budget: SourceResourceBudget::session_default(),
    })
    .map_err(|error| error.to_string())
}
