use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::ComponentHandle;
use tokio::task::JoinHandle;

use super::search::{
    advance_search_generation, reset_search_state, start_catalog_search, start_search,
};
use super::{Frontend, LauncherWindow, UiWeak, lock};
use crate::ipc::LauncherState;
use crate::search_setup::{SearchSetup, SetupSnapshot};

#[derive(PartialEq, Eq)]
struct Presentation {
    snapshot: Arc<SetupSnapshot>,
    selected: Option<Arc<PathBuf>>,
    busy: bool,
    configured: bool,
    picker_error: Option<Arc<str>>,
}

/// Keeps approval in the host, never in renderer-supplied path text.
pub(super) struct SearchSetupController {
    backend: SearchSetup,
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    selected: Mutex<Option<Arc<PathBuf>>>,
    picker_error: Mutex<Option<Arc<str>>>,
    presentation: Mutex<Option<Presentation>>,
    busy: AtomicBool,
    stopping: AtomicBool,
    presentation_revision: AtomicU64,
    restart_query: AtomicBool,
    poll: Mutex<Option<JoinHandle<()>>>,
    action: Mutex<Option<JoinHandle<()>>>,
    view: Mutex<Option<JoinHandle<()>>>,
}

impl SearchSetupController {
    pub(super) fn install(
        ui: &LauncherWindow,
        state: Arc<LauncherState>,
        frontend: Arc<Frontend>,
        runtime: tokio::runtime::Handle,
    ) -> Arc<Self> {
        let controller = Arc::new(Self {
            backend: SearchSetup::new(),
            state,
            frontend,
            runtime,
            ui: ui.as_weak(),
            selected: Mutex::new(None),
            picker_error: Mutex::new(None),
            presentation: Mutex::new(None),
            busy: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            presentation_revision: AtomicU64::new(0),
            restart_query: AtomicBool::new(false),
            poll: Mutex::new(None),
            action: Mutex::new(None),
            view: Mutex::new(None),
        });
        let chooser = Arc::clone(&controller);
        ui.on_search_folder_requested(move || chooser.choose_folder());
        let enabler = Arc::clone(&controller);
        ui.on_search_enable_requested(move || enabler.enable());
        let disabler = Arc::clone(&controller);
        ui.on_search_disable_requested(move || disabler.disable());
        controller.publish(false);
        let poller = Arc::clone(&controller);
        *lock(&controller.poll) = Some(controller.runtime.spawn(async move {
            poller.poll().await;
        }));
        let viewer = Arc::clone(&controller);
        *lock(&controller.view) = Some(controller.runtime.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if viewer.stopping.load(Ordering::Acquire) {
                    break;
                }
                viewer.publish(false);
            }
        }));
        controller
    }

    fn begin(self: &Arc<Self>) -> bool {
        if self.stopping.load(Ordering::Acquire) || self.busy.swap(true, Ordering::AcqRel) {
            return false;
        }
        *lock(&self.picker_error) = None;
        self.publish(false);
        true
    }

    fn choose_folder(self: &Arc<Self>) {
        let Some(window) = self.ui.upgrade() else {
            return;
        };
        if !self.begin() {
            return;
        }
        if let Err(error) = self.state.begin_modal() {
            self.finish_picker(Err(error.message));
            return;
        }
        #[cfg(target_os = "linux")]
        let parent = crate::platform::window_identifier(window.window(), &self.runtime);
        #[cfg(not(target_os = "linux"))]
        let parent = None;
        let controller = Arc::clone(self);
        *lock(&self.action) = Some(self.runtime.spawn(async move {
            let selection = super::platform::choose_folder(parent).await;
            controller.state.end_modal();
            let selected = match selection {
                Ok(Some(root)) => {
                    let (invalidated, applied) = tokio::sync::oneshot::channel();
                    let invalidator = Arc::clone(&controller);
                    let _ = slint::invoke_from_event_loop(move || {
                        invalidator.invalidate_documents();
                        let _ = invalidated.send(());
                    });
                    if applied.await.is_err() {
                        return;
                    }
                    // Only the owned approval is retired before a replacement
                    // folder is confirmed. An external connection stays unchanged.
                    let managed = controller
                        .state
                        .settings()
                        .map(|settings| settings.managed_search().is_some());
                    let disconnected = match managed {
                        Ok(true) => controller.backend.disable(&controller.state).await,
                        Ok(false) => Ok(()),
                        Err(error) => Err(error.message),
                    };
                    match disconnected {
                        Ok(()) => {
                            *lock(&controller.selected) = Some(Arc::new(root));
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                }
                Ok(None) => Ok(()),
                Err(error) => Err(error.message),
            };
            controller.finish_picker(selected);
        }));
    }

    fn finish_picker(self: &Arc<Self>, result: Result<(), String>) {
        *lock(&self.picker_error) = result.err().map(Arc::from);
        self.busy.store(false, Ordering::Release);
        self.publish(true);
    }

    fn enable(self: &Arc<Self>) {
        let snapshot = self.backend.snapshot();
        let selected = lock(&self.selected);
        let root = match selected.as_ref() {
            Some(root) => Some(root.as_ref().clone()),
            None => match snapshot.error.as_ref() {
                Some(_) => snapshot.root.clone(),
                None => None,
            },
        };
        drop(selected);
        let Some(root) = root else {
            return;
        };
        if !self.begin() {
            return;
        }
        self.invalidate_documents();
        let controller = Arc::clone(self);
        *lock(&self.action) = Some(self.runtime.spawn(async move {
            if controller
                .backend
                .enable(&controller.state, root)
                .await
                .is_ok()
            {
                *lock(&controller.selected) = None;
            }
            controller.busy.store(false, Ordering::Release);
            controller.publish(true);
        }));
    }

    fn disable(self: &Arc<Self>) {
        if !self.begin() {
            return;
        }
        *lock(&self.selected) = None;
        self.invalidate_documents();
        let controller = Arc::clone(self);
        *lock(&self.action) = Some(self.runtime.spawn(async move {
            let _ = controller.backend.disable(&controller.state).await;
            controller.busy.store(false, Ordering::Release);
            controller.publish(true);
        }));
    }

    fn invalidate_documents(&self) {
        let Some(generation) = advance_search_generation(&self.frontend) else {
            return;
        };
        self.frontend
            .active_search
            .store(generation, Ordering::Release);
        let query = lock(&self.frontend.model).query.clone();
        reset_search_state(&self.frontend, &self.ui, &query, false);
        // Preserve app-only use while the old document authority is torn down.
        drop(start_catalog_search(
            Arc::clone(&self.state),
            Arc::clone(&self.frontend),
            &self.runtime,
            self.ui.clone(),
            query,
            generation,
            None,
        ));
    }

    async fn poll(self: Arc<Self>) {
        let _ = self.backend.resume(&self.state).await;
        self.publish(true);
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if self.stopping.load(Ordering::Acquire) {
                break;
            }
            if self.busy.load(Ordering::Acquire) {
                continue;
            }
            let _ = self.backend.refresh(&self.state).await;
            self.publish(false);
        }
    }

    fn configured(&self) -> bool {
        self.state
            .settings()
            .is_ok_and(|settings| settings.has_search_service())
    }

    fn publish(self: &Arc<Self>, restart_query: bool) {
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        let mut previous = lock(&self.presentation);
        if restart_query {
            self.restart_query.store(true, Ordering::Release);
        }
        let snapshot = self.backend.snapshot();
        let presentation = Presentation {
            busy: self.busy.load(Ordering::Acquire) || snapshot.busy,
            configured: self.configured(),
            snapshot,
            selected: lock(&self.selected).clone(),
            picker_error: lock(&self.picker_error).clone(),
        };
        if !restart_query && previous.as_ref() == Some(&presentation) {
            return;
        }
        if previous
            .as_ref()
            .is_some_and(|previous| previous.configured != presentation.configured)
        {
            self.restart_query.store(true, Ordering::Release);
        }
        let snapshot = Arc::clone(&presentation.snapshot);
        let selected = presentation.selected.clone();
        let busy = presentation.busy;
        let picker_error = presentation.picker_error.clone();
        *previous = Some(presentation);
        let revision = self.presentation_revision.fetch_add(1, Ordering::AcqRel) + 1;
        drop(previous);
        let ui = self.ui.clone();
        let controller = Arc::clone(self);
        let _ = slint::invoke_from_event_loop(move || {
            if controller.stopping.load(Ordering::Acquire)
                || controller.presentation_revision.load(Ordering::Acquire) != revision
            {
                return;
            }
            let Some(window) = ui.upgrade() else {
                return;
            };
            let error = picker_error.as_deref().or(snapshot.error.as_deref());
            let root = selected.as_deref().or(snapshot.root.as_ref());
            window.set_search_setup_root(
                root.map_or_else(String::new, |root| root.display().to_string())
                    .into(),
            );
            window.set_search_setup_enabled(snapshot.enabled && selected.is_none());
            window.set_search_setup_busy(busy);
            window.set_search_setup_consent(
                selected.is_some() || (snapshot.error.is_some() && root.is_some()),
            );
            window.set_search_setup_status(snapshot.status.as_str().into());
            window.set_search_setup_detail(snapshot.detail.as_str().into());
            if let Some(error) = error {
                window.set_search_setup_error(error.into());
            } else {
                window.set_search_setup_error("".into());
            }
            if controller.restart_query.swap(false, Ordering::AcqRel) {
                let query = lock(&controller.frontend.model).query.clone();
                start_search(
                    Arc::clone(&controller.state),
                    Arc::clone(&controller.frontend),
                    controller.runtime.clone(),
                    ui,
                    query,
                    None,
                );
            }
        });
    }

    pub(super) async fn shutdown(&self) -> Result<(), String> {
        self.stopping.store(true, Ordering::Release);
        if let Some(poll) = lock(&self.poll).take() {
            poll.abort();
        }
        if let Some(action) = lock(&self.action).take() {
            action.abort();
        }
        if let Some(view) = lock(&self.view).take() {
            view.abort();
        }
        self.backend.shutdown().await
    }
}
