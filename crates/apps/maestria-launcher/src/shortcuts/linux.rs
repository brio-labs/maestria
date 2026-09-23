use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use tauri::{AppHandle, Manager, WebviewWindow};

use super::ConfigureOutcome;
use crate::errors::LauncherError;
use crate::model::{ShortcutConfigureAction, ShortcutControl, ShortcutStatus};
use crate::platform::{self, DisplayBackend};

mod helpers;
mod portal;
mod state;
mod status;
mod worker;
mod x11;

use self::state::{OperationGuard, ShortcutInner, ShortcutStateInner};
use self::status::{emit_status, lock_state, set_status, unconfigured_status, unsupported_status};

const APP_ID: &str = "io.github.briolabs.Maestria.Launcher";
const SHORTCUT_ID: &str = "activate-launcher";
const DEFAULT_SHORTCUT: &str = "Control+Space";
const PORTAL_DEFAULT_TRIGGER: &str = "CTRL+space";
const SHUTDOWN_IDLE: u8 = 0;
const SHUTDOWN_IN_FLIGHT: u8 = 1;
const SHUTDOWN_COMPLETE: u8 = 2;
const WORKER_CAPACITY: usize = 2;

/// Native global-shortcut lifecycle owned by the resident launcher.
///
/// The object is deliberately small and managed by Tauri. X11 registration is
/// delegated to Tauri's official global-shortcut plugin, while Wayland portal
/// objects stay on one asynchronous worker and never cross into the GTK main
/// thread.
pub struct Shortcuts {
    inner: Arc<ShortcutInner>,
}

impl Shortcuts {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ShortcutInner {
                state: Mutex::new(ShortcutStateInner {
                    app: None,
                    backend: DisplayBackend::Unavailable,
                    initialized: false,
                    plugin_installed: false,
                    x11_shortcut: None,
                    portal_worker: None,
                    registered_owner: None,
                    status: unsupported_status("Global shortcut support is not initialized"),
                }),
                operation_in_flight: AtomicBool::new(false),
                shutdown_state: AtomicU8::new(SHUTDOWN_IDLE),
            }),
        }
    }

    pub fn status(&self) -> ShortcutStatus {
        lock_state(&self.inner).status.clone()
    }

    fn start_operation(&self) -> Result<OperationGuard, LauncherError> {
        if self.inner.shutdown_state.load(Ordering::Acquire) != SHUTDOWN_IDLE {
            return Err(LauncherError::new(
                "shortcut_unavailable",
                "shortcut lifecycle is shutting down",
                true,
            ));
        }
        self.inner
            .operation_in_flight
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| OperationGuard(Arc::clone(&self.inner)))
            .map_err(|_| {
                LauncherError::new(
                    "shortcut_unavailable",
                    "another shortcut configuration request is in progress",
                    true,
                )
            })
    }

    fn begin_shutdown(&self) -> bool {
        self.inner
            .shutdown_state
            .compare_exchange(
                SHUTDOWN_IDLE,
                SHUTDOWN_IN_FLIGHT,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn complete_shutdown(&self) {
        self.inner
            .shutdown_state
            .store(SHUTDOWN_COMPLETE, Ordering::Release);
    }

    fn shutdown_complete(&self) -> bool {
        self.inner.shutdown_state.load(Ordering::Acquire) == SHUTDOWN_COMPLETE
    }
}

/// Detect the realized display backend and install the X11 plugin only when
/// the actual native handles prove an X11 display. No shortcut is configured.
pub fn initialize(app: &AppHandle, window: &WebviewWindow) -> Result<(), LauncherError> {
    let shortcuts = app
        .try_state::<Shortcuts>()
        .ok_or_else(|| LauncherError::platform_unavailable("shortcut state is unavailable"))?;
    if lock_state(&shortcuts.inner).initialized {
        return Ok(());
    }
    let backend = platform::display_backend(window);
    let install_plugin = {
        let mut state = lock_state(&shortcuts.inner);
        if state.initialized {
            return Ok(());
        }
        state.initialized = true;
        state.backend = backend;
        state.app = Some(app.clone());
        state.status = match backend {
            DisplayBackend::X11 => unconfigured_status(
                ShortcutControl::Application,
                DEFAULT_SHORTCUT,
                None,
                ShortcutConfigureAction::Setup,
            ),
            DisplayBackend::Wayland => unconfigured_status(
                ShortcutControl::System,
                DEFAULT_SHORTCUT,
                None,
                ShortcutConfigureAction::Setup,
            ),
            DisplayBackend::Unavailable => {
                unsupported_status("The realized native display has no supported shortcut backend")
            }
        };
        backend == DisplayBackend::X11 && !state.plugin_installed
    };

    let should_emit = if install_plugin {
        x11::install_plugin(app, &shortcuts.inner)?
    } else {
        true
    };
    if should_emit {
        emit_status(app, &shortcuts.status());
    }

    Ok(())
}

/// Configure the current backend. Calls are serialized, including automatic
/// restoration and deliberate setup/change actions.
///
/// # Cancellation
///
/// Cancellation during a Linux portal request resolves as a status outcome.
/// A valid v2 session remains live after a cancelled reconfiguration; failed
/// setup sessions are closed and the worker remains lifecycle-managed.
pub async fn configure(
    app: AppHandle,
    preferred: String,
    explicit: bool,
) -> Result<ConfigureOutcome, LauncherError> {
    helpers::validate_accelerator(&preferred)?;
    let shortcuts = app
        .try_state::<Shortcuts>()
        .ok_or_else(|| LauncherError::platform_unavailable("shortcut state is unavailable"))?;
    let _operation = shortcuts.start_operation()?;
    let (backend, plugin_installed) = {
        let state = lock_state(&shortcuts.inner);
        (state.backend, state.plugin_installed)
    };
    match backend {
        DisplayBackend::X11 if !plugin_installed => Ok(ConfigureOutcome {
            status: shortcuts.status(),
            denied: false,
        }),
        DisplayBackend::X11 => x11::configure(&app, &shortcuts.inner, preferred).await,
        DisplayBackend::Wayland => {
            worker::configure_wayland(&app, &shortcuts.inner, preferred, explicit).await
        }
        DisplayBackend::Unavailable => Ok(ConfigureOutcome {
            status: shortcuts.status(),
            denied: false,
        }),
    }
}

/// Release the active registration while retaining initialized backend/plugin
/// state so a later explicit setup can configure it again.
pub fn clear(app: &AppHandle) -> Result<(), LauncherError> {
    let shortcuts = app
        .try_state::<Shortcuts>()
        .ok_or_else(|| LauncherError::platform_unavailable("shortcut state is unavailable"))?;
    let _operation = shortcuts.start_operation()?;
    let state = lock_state(&shortcuts.inner);
    let backend = state.backend;
    let plugin_installed = state.plugin_installed;
    drop(state);

    match backend {
        DisplayBackend::X11 if !plugin_installed => Ok(()),
        DisplayBackend::X11 => x11::clear(app, &shortcuts.inner),
        DisplayBackend::Wayland => {
            let worker = lock_state(&shortcuts.inner).portal_worker.take();
            if let Some(mut worker) = worker
                && let Some(shutdown) = worker.shutdown.take()
            {
                let _ = shutdown.send(());
            }
            let status = unconfigured_status(
                ShortcutControl::System,
                DEFAULT_SHORTCUT,
                None,
                ShortcutConfigureAction::Setup,
            );
            set_status(&shortcuts.inner, status);
            Ok(())
        }
        DisplayBackend::Unavailable => Ok(()),
    }
}

/// Release registrations and ask the portal worker to close its session.
pub fn shutdown(app: &AppHandle) -> Result<(), LauncherError> {
    let shortcuts = app
        .try_state::<Shortcuts>()
        .ok_or_else(|| LauncherError::platform_unavailable("shortcut state is unavailable"))?;
    let (backend, plugin_installed, current, worker) = {
        let mut state = lock_state(&shortcuts.inner);
        (
            state.backend,
            state.plugin_installed,
            state.x11_shortcut.take(),
            state.portal_worker.take(),
        )
    };

    match backend {
        DisplayBackend::X11 if !plugin_installed => Ok(()),
        DisplayBackend::X11 => x11::shutdown(app, current),
        DisplayBackend::Wayland => {
            if let Some(worker) = worker {
                worker::wait_for_portal_worker(worker);
            }
            Ok(())
        }
        DisplayBackend::Unavailable => Ok(()),
    }
}

pub fn request_shutdown(app: &AppHandle) -> Result<bool, LauncherError> {
    let shortcuts = app
        .try_state::<Shortcuts>()
        .ok_or_else(|| LauncherError::platform_unavailable("shortcut state is unavailable"))?;
    if !shortcuts.begin_shutdown() {
        return Ok(false);
    }
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = shutdown(&handle) {
            eprintln!("Shortcut shutdown: {}", error.message);
        }
        if let Some(shortcuts) = handle.try_state::<Shortcuts>() {
            shortcuts.complete_shutdown();
        }
        handle.exit(0);
    });
    Ok(true)
}

pub fn shutdown_complete(app: &AppHandle) -> bool {
    app.try_state::<Shortcuts>()
        .is_some_and(|shortcuts| shortcuts.shutdown_complete())
}
