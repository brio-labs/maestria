use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use futures_util::Stream;
use tauri::AppHandle;
use tokio::sync::{Notify, mpsc, oneshot};

use super::ConfigureOutcome;
use crate::model::ShortcutStatus;
use crate::platform::DisplayBackend;

pub(super) struct ShortcutInner {
    pub(super) state: Mutex<ShortcutStateInner>,
    pub(super) operation_in_flight: AtomicBool,
    pub(super) shutdown_state: AtomicU8,
}

pub(super) struct ShortcutStateInner {
    pub(super) app: Option<AppHandle>,
    pub(super) backend: DisplayBackend,
    pub(super) initialized: bool,
    pub(super) plugin_installed: bool,
    pub(super) x11_shortcut: Option<String>,
    pub(super) portal_worker: Option<PortalWorker>,
    pub(super) registered_owner: Option<String>,
    pub(super) status: ShortcutStatus,
}

pub(super) type ExportedParent = Arc<ashpd::WindowIdentifier>;
pub(super) type ParentReceiver = oneshot::Receiver<Option<ExportedParent>>;

pub(super) struct PortalWorker {
    pub(super) sender: mpsc::Sender<PortalCommand>,
    pub(super) shutdown: Option<oneshot::Sender<()>>,
    pub(super) completion: Arc<Notify>,
}

pub(super) enum PortalCommand {
    Configure {
        preferred: String,
        explicit: bool,
        parent: ParentReceiver,
        response: oneshot::Sender<ConfigureOutcome>,
    },
}

pub(super) enum PortalEvent {
    Command(Option<PortalCommand>),
    Activated(Option<ashpd::desktop::global_shortcuts::Activated>),
    Changed(Option<ashpd::desktop::global_shortcuts::ShortcutsChanged>),
    Closed,
    Owner(Result<Option<ashpd::zbus::Message>, ashpd::zbus::Error>),
    Shutdown,
}

pub(super) type ActivatedStream =
    Pin<Box<dyn Stream<Item = ashpd::desktop::global_shortcuts::Activated> + Send + 'static>>;
pub(super) type ChangedStream = Pin<
    Box<dyn Stream<Item = ashpd::desktop::global_shortcuts::ShortcutsChanged> + Send + 'static>,
>;
pub(super) type ClosedStream = Pin<Box<dyn Stream<Item = ()> + Send + 'static>>;
pub(super) type Portal = ashpd::desktop::global_shortcuts::GlobalShortcuts<'static>;
pub(super) type PortalSession = ashpd::desktop::Session<'static, Portal>;

pub(super) struct PortalSessionState {
    pub(super) portal: Portal,
    pub(super) version: u32,
    pub(super) owner_name: String,
    pub(super) session_path: String,
    pub(super) session: PortalSession,
    pub(super) activated: ActivatedStream,
    pub(super) changed: ChangedStream,
    pub(super) closed: ClosedStream,
    pub(super) owner: ashpd::zbus::MessageStream,
    pub(super) bound: bool,
    pub(super) trigger_description: Option<String>,
}

pub(super) struct OperationGuard(pub(super) Arc<ShortcutInner>);

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.0.operation_in_flight.store(false, Ordering::Release);
    }
}

pub(super) struct CompletionGuard(pub(super) Arc<Notify>);

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}
