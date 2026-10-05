use std::sync::Arc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use super::{Controller, LauncherWindow, StoreState, Utilities, UtilityKind, clear_editor};

pub(super) fn install(ui: &LauncherWindow, controller: Arc<Controller>) {
    let host = Arc::clone(&controller);
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(1),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            // The event loop must never wait for a filesystem mutation to release this lock.
            let expired = if let Ok(mut guard) = host.store.try_lock()
                && let StoreState::Ready(store) = &mut *guard
            {
                clipboard_view_expired(&ui, store)
            } else {
                false
            };
            if expired && ui.get_utility_kind().as_str() == "clipboard" {
                // Discard already queued snapshots as well as the visible editor and rows.
                host.cancel();
                ui.set_utility_busy(false);
                clear_editor(&ui);
                ui.set_utility_rows(ModelRc::new(VecModel::default()));
                if ui.get_utilities_open() {
                    host.refresh(&ui);
                }
            }
        },
    );
    let weak = ui.as_weak();
    ui.on_utilities_closed(move || {
        let _timer = &timer;
        controller.cancel();
        if let Some(ui) = weak.upgrade() {
            ui.set_utility_busy(false);
            clear_editor(&ui);
            ui.set_utility_rows(ModelRc::new(VecModel::default()));
        }
    });
}

fn clipboard_view_expired(ui: &LauncherWindow, store: &mut Utilities) -> bool {
    let expired = store.purge_expired();
    if ui.get_utility_kind().as_str() != "clipboard" {
        return expired;
    }
    let selected = ui.get_utility_selected_id();
    if !selected.is_empty()
        && store
            .entry(UtilityKind::Clipboard, selected.as_str())
            .is_err()
    {
        return true;
    }
    // Another access may already have purged the store; UI copies must expire too.
    let rows = ui.get_utility_rows();
    expired
        || (0..rows.row_count()).any(|index| {
            rows.row_data(index).is_some_and(|row| {
                store
                    .entry(UtilityKind::Clipboard, row.id.as_str())
                    .is_err()
            })
        })
}
