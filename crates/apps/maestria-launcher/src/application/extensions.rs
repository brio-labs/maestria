use std::sync::Arc;

use self::callbacks::{Controller, PendingInstall, set_error};
use super::{LauncherWindow, UiWeak, lock};
use crate::ipc::LauncherState;

mod broker;
mod callbacks;
mod form;
mod invoke;
mod management;
mod sandbox;
mod transport;
mod view;
pub(super) fn install_callbacks(
    ui: &LauncherWindow,
    state: Arc<LauncherState>,
    runtime: tokio::runtime::Handle,
) {
    callbacks::install_callbacks(ui, state, runtime);
}
