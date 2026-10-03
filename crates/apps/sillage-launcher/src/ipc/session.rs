use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::catalog::Catalog;
use crate::errors::LauncherError;
use crate::model::{
    HOST_COPY_ACTIVATION, HOST_OPEN_FILE, HOST_PREFERENCES, HOST_QUIT, HOST_REFRESH_APPLICATIONS,
    HOST_RESET_PREFERENCES, ResultKind, SELECTED_FILE_PREFIX, SearchResponse, SearchResult,
};
use crate::settings::SettingsManager;

use super::{ActionTarget, QueryWorker};

const MAX_QUERY_BYTES: usize = 4096;

#[derive(Debug, Clone)]
struct SelectedFile {
    result_id: String,
    path: PathBuf,
    generation: u64,
}

#[derive(Default)]
struct SessionState {
    ready: bool,
    pending_activation: bool,
    last_generation: u64,
    accepted: Vec<SearchResult>,
    selected_file: Option<SelectedFile>,
    selected_file_counter: u64,
    preferences_scope: bool,
    modal_depth: u32,
    action_in_flight: bool,
    allow_exit: bool,
}

/// Native-owned lifecycle and IPC state. The renderer can only address accepted IDs.
pub struct LauncherState {
    session: Mutex<SessionState>,
    settings: Mutex<SettingsManager>,
    query_worker: QueryWorker,
    catalog: Arc<Catalog>,
}

impl LauncherState {
    pub fn new(settings: SettingsManager) -> Result<Self, LauncherError> {
        Ok(Self {
            session: Mutex::new(SessionState::default()),
            settings: Mutex::new(settings),
            query_worker: QueryWorker::new()?,
            catalog: Arc::new(Catalog::default()),
        })
    }

    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }

    pub(crate) fn ensure_generation(&self, generation: u64) -> Result<(), LauncherError> {
        if self.session()?.last_generation != generation {
            return Err(LauncherError::stale_result(
                "A newer activation or query superseded this action",
            ));
        }
        Ok(())
    }

    pub fn initialize_settings(&self, settings: SettingsManager) -> Result<(), LauncherError> {
        *self.settings()? = settings;
        Ok(())
    }

    fn session(&self) -> Result<MutexGuard<'_, SessionState>, LauncherError> {
        self.session.lock().map_err(|_| {
            LauncherError::platform_unavailable("The launcher lifecycle state is unavailable")
        })
    }

    pub(crate) fn settings(&self) -> Result<MutexGuard<'_, SettingsManager>, LauncherError> {
        self.settings.lock().map_err(|_| {
            LauncherError::settings_failed("The launcher preferences state is unavailable")
        })
    }

    pub fn begin_modal(&self) -> Result<(), LauncherError> {
        let mut state = self.session()?;
        state.modal_depth = state.modal_depth.saturating_add(1);
        Ok(())
    }

    pub fn end_modal(&self) {
        if let Ok(mut state) = self.session.lock() {
            state.modal_depth = state.modal_depth.saturating_sub(1);
        }
    }

    pub fn request_activation(&self) -> Result<Option<u64>, LauncherError> {
        let mut state = self.session()?;
        let generation = state.last_generation.checked_add(1).ok_or_else(|| {
            LauncherError::platform_unavailable("Activation generation exhausted")
        })?;
        state.last_generation = generation;
        state.accepted.clear();
        state.selected_file = None;
        state.preferences_scope = false;
        if state.ready {
            Ok(Some(generation))
        } else {
            state.pending_activation = true;
            Ok(None)
        }
    }

    pub fn mark_ready(&self) -> Result<u64, LauncherError> {
        let mut state = self.session()?;
        if state.ready {
            return Err(LauncherError::invalid_request(
                "The launcher is already ready",
            ));
        }
        let generation = state.last_generation.checked_add(1).ok_or_else(|| {
            LauncherError::platform_unavailable("Activation generation exhausted")
        })?;
        state.ready = true;
        state.pending_activation = false;
        state.last_generation = generation;
        state.accepted.clear();
        state.preferences_scope = false;
        Ok(generation)
    }

    pub fn allow_exit(&self) -> Result<(), LauncherError> {
        let mut state = self.session()?;
        state.allow_exit = true;
        Ok(())
    }

    pub fn should_prevent_exit(&self) -> bool {
        match self.session.lock() {
            Ok(state) => !state.allow_exit,
            Err(_) => true,
        }
    }

    pub fn hide_requested(&self) -> Result<bool, LauncherError> {
        Ok(self.session()?.modal_depth == 0)
    }

    pub fn preferences_active(&self) -> Result<bool, LauncherError> {
        Ok(self.session()?.preferences_scope)
    }

    /// Submit a search and retain only the result set for its current generation.
    ///
    /// # Cancellation
    ///
    /// A newer generation invalidates this request before or after worker evaluation.
    /// Dropping the future does not stop the bounded worker, and it leaves no accepted
    /// result set for this call; native effects are not performed by this method.
    pub async fn search(
        &self,
        query: String,
        generation: u64,
    ) -> Result<SearchResponse, LauncherError> {
        {
            let mut state = self.session()?;
            if query.len() > MAX_QUERY_BYTES {
                return Err(LauncherError::invalid_request(
                    "Search queries cannot exceed 4096 UTF-8 bytes",
                ));
            }
            if generation <= state.last_generation {
                return Err(LauncherError::stale_result(
                    "This search generation is no longer current",
                ));
            }
            state.preferences_scope = false;
            state.last_generation = generation;
            state.accepted.clear();
            state.selected_file = None;
        }

        let response = self
            .query_worker
            .submit(query, generation, self.catalog.snapshot()?)
            .await?;
        let mut state = self.session()?;
        if state.last_generation != generation {
            return Err(LauncherError::stale_result(
                "A newer search generation is already active",
            ));
        }
        state.accepted = response.results.clone();
        Ok(response)
    }

    pub fn begin_action(
        &self,
        result_id: &str,
        action_id: &str,
        generation: u64,
    ) -> Result<ActionTarget, LauncherError> {
        let mut state = self.session()?;
        if state.action_in_flight {
            return Err(LauncherError::invalid_request(
                "An action is already in progress",
            ));
        }
        if generation != state.last_generation {
            return Err(LauncherError::stale_result(
                "This result belongs to an older search generation",
            ));
        }
        let target = if state.preferences_scope {
            Self::preference_target(&state, result_id, action_id)?
        } else {
            Self::result_target(&state, result_id, action_id)?
        };
        state.action_in_flight = true;
        Ok(target)
    }

    /// Reserve one authenticated external-evidence action for the active generation.
    pub(crate) fn begin_passage_action(&self, generation: u64) -> Result<(), LauncherError> {
        let mut state = self.session()?;
        if state.action_in_flight {
            return Err(LauncherError::invalid_request(
                "An action is already in progress",
            ));
        }
        if generation != state.last_generation {
            return Err(LauncherError::stale_result(
                "This passage belongs to an older search generation",
            ));
        }
        if state.preferences_scope {
            return Err(LauncherError::invalid_request(
                "Passage actions are unavailable while preferences are open",
            ));
        }
        state.action_in_flight = true;
        Ok(())
    }
    fn preference_target(
        state: &SessionState,
        result_id: &str,
        action_id: &str,
    ) -> Result<ActionTarget, LauncherError> {
        match (result_id, action_id) {
            (HOST_PREFERENCES, HOST_COPY_ACTIVATION) => Ok(ActionTarget::CopyActivation),
            (HOST_PREFERENCES, HOST_RESET_PREFERENCES) => Ok(ActionTarget::ResetPreferences),
            (id, _) if id.starts_with(SELECTED_FILE_PREFIX) => {
                let selected = state.selected_file.as_ref().ok_or_else(|| {
                    LauncherError::stale_result("The selected file is no longer available")
                })?;
                if selected.result_id != id {
                    return Err(LauncherError::stale_result(
                        "The selected file is no longer available",
                    ));
                }
                match action_id {
                    "file.open" => Ok(ActionTarget::OpenSelectedFile(selected.path.clone())),
                    "file.copy-path" => Ok(ActionTarget::CopySelectedPath {
                        path: selected.path.clone(),
                        result_id: selected.result_id.clone(),
                    }),
                    _ => Err(LauncherError::invalid_request("Action is not available")),
                }
            }
            _ => Err(LauncherError::invalid_request("Action is not available")),
        }
    }

    fn result_target(
        state: &SessionState,
        result_id: &str,
        action_id: &str,
    ) -> Result<ActionTarget, LauncherError> {
        let result = state
            .accepted
            .iter()
            .find(|result| result.id == result_id)
            .ok_or_else(|| {
                LauncherError::stale_result("The requested result is no longer available")
            })?;
        if !result.actions.iter().any(|action| action.id == action_id) {
            return Err(LauncherError::invalid_request("Action is not available"));
        }
        match (result_id, action_id) {
            (HOST_OPEN_FILE, "open") => Ok(ActionTarget::OpenFile),
            (HOST_PREFERENCES, "open") => Ok(ActionTarget::ShowPreferences),
            (HOST_REFRESH_APPLICATIONS, "refresh") => Ok(ActionTarget::Refresh),
            (HOST_QUIT, "quit") => Ok(ActionTarget::Quit),
            ("calculation", "copy-result") if matches!(result.kind, ResultKind::Calculation) => {
                Ok(ActionTarget::CopyValue(result.title.clone()))
            }
            (id, "open") if matches!(result.kind, ResultKind::Application) => {
                let desktop_id = id
                    .strip_prefix("app:")
                    .ok_or_else(|| LauncherError::invalid_request("Invalid application ID"))?;
                Ok(ActionTarget::OpenApplication(desktop_id.to_string()))
            }
            (_, "copy-name") if matches!(result.kind, ResultKind::Application) => {
                Ok(ActionTarget::CopyValue(result.title.clone()))
            }
            _ => Err(LauncherError::invalid_request("Action is not available")),
        }
    }

    pub fn finish_action(&self) {
        if let Ok(mut state) = self.session.lock() {
            state.action_in_flight = false;
        }
    }

    pub fn enter_preferences(&self) -> Result<(), LauncherError> {
        let mut state = self.session()?;
        state.preferences_scope = true;
        Ok(())
    }
    pub fn leave_preferences(&self) -> Result<(), LauncherError> {
        let mut state = self.session()?;
        if state.selected_file.is_none() {
            state.preferences_scope = false;
        }
        Ok(())
    }

    pub fn dismiss(&self) -> Result<(), LauncherError> {
        let mut state = self.session()?;
        state.accepted.clear();
        state.selected_file = None;
        state.preferences_scope = false;
        state.pending_activation = false;
        Ok(())
    }

    pub(crate) fn install_selected_file(
        &self,
        path: PathBuf,
        generation: u64,
    ) -> Result<(String, String), LauncherError> {
        let mut state = self.session()?;
        if state.last_generation != generation {
            return Err(LauncherError::stale_result(
                "The file selection belongs to an older activation",
            ));
        }
        state.selected_file_counter = state
            .selected_file_counter
            .checked_add(1)
            .ok_or_else(|| LauncherError::platform_unavailable("File selection limit reached"))?;
        let result_id = format!("{SELECTED_FILE_PREFIX}{}", state.selected_file_counter);
        let display_path = path.display().to_string();
        state.selected_file = Some(SelectedFile {
            result_id: result_id.clone(),
            path,
            generation,
        });
        state.preferences_scope = true;
        Ok((result_id, display_path))
    }

    /// Keep the accepted file generation stable until native dispatch completes.
    pub(crate) fn with_current_file<T>(
        &self,
        generation: u64,
        path: &Path,
        dispatch: impl FnOnce() -> Result<T, LauncherError>,
    ) -> Result<T, LauncherError> {
        let state = self.session()?;
        if state.last_generation != generation
            || !state
                .selected_file
                .as_ref()
                .is_some_and(|selected| selected.generation == generation && selected.path == path)
        {
            return Err(LauncherError::stale_result(
                "The selected file belongs to an older activation",
            ));
        }
        dispatch()
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
