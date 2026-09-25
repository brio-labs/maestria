use std::sync::{Arc, Mutex, mpsc};

use global_hotkey::hotkey::HotKey;
use tokio::sync::{mpsc as async_mpsc, oneshot};

use crate::errors::LauncherError;
use crate::model::{ShortcutConfigureAction, ShortcutControl, ShortcutState, ShortcutStatus};
use crate::platform::{self, DisplayBackend};

mod portal;
mod x11;
pub(super) use x11::parse_hotkey;

pub(super) const APP_ID: &str = "io.github.briolabs.Maestria.Launcher";
pub(super) const SHORTCUT_ID: &str = "activate-launcher";
pub(super) const DEFAULT_SHORTCUT: &str = "Control+Space";
pub(super) const PORTAL_DEFAULT_TRIGGER: &str = "CTRL+space";

pub struct Shortcuts {
    status: Arc<Mutex<ShortcutStatus>>,
    activation: mpsc::SyncSender<()>,
    display: Mutex<DisplayBackend>,
    x11: Mutex<Option<async_mpsc::Sender<X11Command>>>,
    portal: Mutex<Option<async_mpsc::Sender<PortalCommand>>>,
    parent: Mutex<Option<Arc<ashpd::WindowIdentifier>>>,
}

pub(super) enum X11Command {
    Configure {
        hotkey: HotKey,
        response: oneshot::Sender<Result<ShortcutStatus, LauncherError>>,
    },
    Clear {
        response: oneshot::Sender<Result<ShortcutStatus, LauncherError>>,
    },
    Shutdown,
}

pub(super) enum PortalCommand {
    Configure {
        preferred: String,
        explicit: bool,
        parent: Option<Arc<ashpd::WindowIdentifier>>,
        response: oneshot::Sender<ShortcutStatus>,
    },
    Clear {
        response: oneshot::Sender<ShortcutStatus>,
    },
    Shutdown,
}

impl Shortcuts {
    pub fn new(activation: mpsc::SyncSender<()>) -> Self {
        let status = ShortcutStatus {
            state: ShortcutState::Unconfigured,
            description: "Shortcut not configured".to_string(),
            message: None,
            control: ShortcutControl::Unavailable,
            configure_action: ShortcutConfigureAction::Setup,
        };
        Self {
            status: Arc::new(Mutex::new(status)),
            activation,
            display: Mutex::new(DisplayBackend::Unavailable),
            x11: Mutex::new(None),
            portal: Mutex::new(None),
            parent: Mutex::new(None),
        }
    }

    pub fn initialize(
        &self,
        window: &slint::Window,
        runtime: &tokio::runtime::Handle,
    ) -> Result<(), LauncherError> {
        let backend = platform::detect_display_backend(window)?;
        *lock(&self.display) = backend;
        *lock(&self.parent) = platform::window_identifier(window, runtime).map(Arc::new);
        match backend {
            DisplayBackend::X11 => self.initialize_x11()?,
            DisplayBackend::Wayland => self.initialize_portal()?,
            DisplayBackend::Unavailable => self.set_status(unsupported_status(
                "The window system does not expose a supported global shortcut backend",
            )),
        }
        Ok(())
    }

    fn initialize_x11(&self) -> Result<(), LauncherError> {
        self.set_status(unconfigured_status(
            ShortcutControl::Application,
            DEFAULT_SHORTCUT,
            None,
            ShortcutConfigureAction::Setup,
        ));
        let (sender, receiver) = async_mpsc::channel(8);
        *lock(&self.x11) = Some(sender);
        let status = Arc::clone(&self.status);
        let activation = self.activation.clone();
        std::thread::Builder::new()
            .name("maestria-launcher-shortcuts".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        runtime.block_on(x11::x11_worker(receiver, status, activation));
                    }
                    Err(error) => eprintln!("X11 shortcut runtime failed: {error}"),
                }
            })
            .map_err(|error| {
                LauncherError::platform_unavailable(format!(
                    "X11 shortcut worker could not start: {error}"
                ))
            })?;
        Ok(())
    }

    fn initialize_portal(&self) -> Result<(), LauncherError> {
        self.set_status(unconfigured_status(
            ShortcutControl::System,
            DEFAULT_SHORTCUT,
            Some("The desktop controls shortcut approval and activation."),
            ShortcutConfigureAction::Setup,
        ));
        let (sender, receiver) = async_mpsc::channel(2);
        *lock(&self.portal) = Some(sender);
        let status = Arc::clone(&self.status);
        let activation = self.activation.clone();
        std::thread::Builder::new()
            .name("maestria-launcher-portal".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        runtime.block_on(portal::portal_worker(receiver, status, activation));
                    }
                    Err(error) => eprintln!("Global shortcut portal runtime failed: {error}"),
                }
            })
            .map_err(|error| {
                LauncherError::platform_unavailable(format!(
                    "Global shortcut portal worker could not start: {error}"
                ))
            })?;
        Ok(())
    }

    pub fn status(&self) -> ShortcutStatus {
        lock(&self.status).clone()
    }

    /// Configure the application or desktop shortcut after validating the requested key.
    ///
    /// # Cancellation
    /// If cancellation happens before a command is sent, the backend is unchanged. Once sent,
    /// its worker may complete configuration and update status after this future is dropped.
    pub async fn configure(
        &self,
        preferred: &str,
        explicit: bool,
    ) -> Result<ShortcutStatus, LauncherError> {
        x11::validate_accelerator(preferred).map_err(LauncherError::invalid_request)?;
        let display = *lock(&self.display);
        match display {
            DisplayBackend::X11 => {
                let hotkey =
                    x11::parse_hotkey(preferred).map_err(LauncherError::invalid_request)?;
                let sender = lock(&self.x11).clone().ok_or_else(|| {
                    LauncherError::platform_unavailable("X11 shortcut manager is unavailable")
                })?;
                let (response, receiver) = oneshot::channel();
                sender
                    .send(X11Command::Configure { hotkey, response })
                    .await
                    .map_err(|_| {
                        LauncherError::platform_unavailable("X11 shortcut worker stopped")
                    })?;
                let status = receiver.await.map_err(|_| {
                    LauncherError::platform_unavailable("X11 shortcut worker stopped")
                })??;
                self.set_status(status.clone());
                Ok(status)
            }
            DisplayBackend::Wayland => {
                let parent = lock(&self.parent).clone();
                let sender = lock(&self.portal).clone().ok_or_else(|| {
                    LauncherError::platform_unavailable(
                        "The global-shortcuts portal is unavailable",
                    )
                })?;
                let (response, receiver) = oneshot::channel();
                sender
                    .send(PortalCommand::Configure {
                        preferred: preferred.to_string(),
                        explicit,
                        parent,
                        response,
                    })
                    .await
                    .map_err(|_| {
                        LauncherError::platform_unavailable(
                            "The global-shortcuts portal disconnected",
                        )
                    })?;
                let status = receiver.await.map_err(|_| {
                    LauncherError::platform_unavailable("The global-shortcuts portal disconnected")
                })?;
                self.set_status(status.clone());
                Ok(status)
            }
            DisplayBackend::Unavailable => Ok(self.status()),
        }
    }

    /// Clear the application or desktop shortcut registration.
    ///
    /// # Cancellation
    /// If cancellation happens before a clear command is sent, the registration remains intact.
    /// Once sent, its worker may clear the registration and update status after cancellation.
    pub async fn clear(&self) -> Result<ShortcutStatus, LauncherError> {
        let display = *lock(&self.display);
        match display {
            DisplayBackend::X11 => {
                let sender = lock(&self.x11).clone().ok_or_else(|| {
                    LauncherError::platform_unavailable("X11 shortcut manager is unavailable")
                })?;
                let (response, receiver) = oneshot::channel();
                sender
                    .send(X11Command::Clear { response })
                    .await
                    .map_err(|_| {
                        LauncherError::platform_unavailable("X11 shortcut worker stopped")
                    })?;
                let status = receiver.await.map_err(|_| {
                    LauncherError::platform_unavailable("X11 shortcut worker stopped")
                })??;
                self.set_status(status.clone());
                Ok(status)
            }
            DisplayBackend::Wayland => {
                let sender = lock(&self.portal).clone().ok_or_else(|| {
                    LauncherError::platform_unavailable(
                        "The global-shortcuts portal is unavailable",
                    )
                })?;
                let (response, receiver) = oneshot::channel();
                sender
                    .send(PortalCommand::Clear { response })
                    .await
                    .map_err(|_| {
                        LauncherError::platform_unavailable(
                            "The global-shortcuts portal disconnected",
                        )
                    })?;
                let status = receiver.await.map_err(|_| {
                    LauncherError::platform_unavailable("The global-shortcuts portal disconnected")
                })?;
                self.set_status(status.clone());
                Ok(status)
            }
            DisplayBackend::Unavailable => Ok(self.status()),
        }
    }

    pub fn shutdown(&self) -> Result<(), LauncherError> {
        if let Some(sender) = lock(&self.portal).as_ref() {
            let _ = sender.try_send(PortalCommand::Shutdown);
        }
        if let Some(sender) = lock(&self.x11).as_ref() {
            let _ = sender.try_send(X11Command::Shutdown);
        }
        Ok(())
    }

    fn set_status(&self, status: ShortcutStatus) {
        *lock(&self.status) = status;
    }
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) fn unconfigured_status(
    control: ShortcutControl,
    description: &str,
    message: Option<&str>,
    configure_action: ShortcutConfigureAction,
) -> ShortcutStatus {
    ShortcutStatus {
        state: ShortcutState::Unconfigured,
        description: description.to_string(),
        message: message.map(ToOwned::to_owned),
        control,
        configure_action,
    }
}

pub(super) fn available_status(
    control: ShortcutControl,
    description: &str,
    message: Option<&str>,
) -> ShortcutStatus {
    ShortcutStatus {
        state: ShortcutState::Available,
        description: description.to_string(),
        message: message.map(ToOwned::to_owned),
        control,
        configure_action: ShortcutConfigureAction::Change,
    }
}

pub(super) fn available_system_status(description: &str, portal_version: u32) -> ShortcutStatus {
    let reported = !description.trim().is_empty();
    let description = if reported {
        description.to_string()
    } else {
        "No key combination reported".to_string()
    };
    let message = (!reported).then(|| {
        concat!(
            "Registered with the desktop. Configure a compositor binding ",
            "if your desktop does not assign one.",
        )
        .to_string()
    });
    ShortcutStatus {
        state: if reported {
            ShortcutState::Available
        } else {
            ShortcutState::Unconfigured
        },
        description,
        message,
        control: ShortcutControl::System,
        configure_action: if portal_version >= 2 {
            ShortcutConfigureAction::Change
        } else {
            ShortcutConfigureAction::Rebind
        },
    }
}

pub(super) fn unavailable_status(message: &str) -> ShortcutStatus {
    ShortcutStatus {
        state: ShortcutState::Unavailable,
        description: "Global shortcut unavailable".to_string(),
        message: Some(message.to_string()),
        control: ShortcutControl::System,
        configure_action: ShortcutConfigureAction::Retry,
    }
}

pub(super) fn unsupported_status(message: &str) -> ShortcutStatus {
    ShortcutStatus {
        control: ShortcutControl::Unavailable,
        ..unavailable_status(message)
    }
}
