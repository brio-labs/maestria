use std::sync::{Arc, MutexGuard};

use tauri::{AppHandle, Emitter};

use super::state::{ShortcutInner, ShortcutStateInner};
use crate::model::{
    LAUNCHER_WINDOW_LABEL, ShortcutConfigureAction, ShortcutControl, ShortcutState, ShortcutStatus,
};

const STATUS_EVENT: &str = "launcher://shortcut-status";

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

pub(super) fn available_system_status(
    description: &str,
    message: Option<&str>,
    portal_version: u32,
) -> ShortcutStatus {
    let reported = !description.trim().is_empty();
    ShortcutStatus {
        state: if reported {
            ShortcutState::Available
        } else {
            ShortcutState::Unconfigured
        },
        description: if reported {
            description.to_string()
        } else {
            "No key combination reported".to_string()
        },
        message: message
            .map(ToOwned::to_owned)
            .or_else(|| {
                (!reported).then_some(
                    "Registered with the desktop. Configure a compositor binding if your desktop does not assign one."
                        .to_string(),
                )
            }),
        control: ShortcutControl::System,
        configure_action: if portal_version >= 2 {
            ShortcutConfigureAction::Change
        } else {
            ShortcutConfigureAction::Rebind
        },
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

pub(super) fn lock_state(inner: &Arc<ShortcutInner>) -> MutexGuard<'_, ShortcutStateInner> {
    match inner.state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) fn set_status(inner: &Arc<ShortcutInner>, status: ShortcutStatus) {
    let app = {
        let mut state = lock_state(inner);
        let changed = state.status.state != status.state
            || state.status.description != status.description
            || state.status.message != status.message
            || state.status.control != status.control
            || state.status.configure_action != status.configure_action;
        state.status = status.clone();
        changed.then(|| state.app.clone()).flatten()
    };
    if let Some(app) = app {
        emit_status(&app, &status);
    }
}

pub(super) fn emit_status(app: &AppHandle, status: &ShortcutStatus) {
    let _ = app.emit_to(LAUNCHER_WINDOW_LABEL, STATUS_EVENT, status);
}
