use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use slint::ComponentHandle;

use super::LauncherWindow;

pub(super) fn install(ui: &LauncherWindow, runtime: tokio::runtime::Handle) {
    ui.set_autostart_busy(true);
    let weak = ui.as_weak();
    runtime.spawn_blocking(move || {
        let result = crate::autostart::enabled(crate::application::platform::config_dir());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_autostart_busy(false);
            match result {
                Ok(enabled) => ui.set_autostart_enabled(enabled),
                Err(error) => ui.set_autostart_message(error.message.into()),
            }
        });
    });
    let revision = Arc::new(AtomicU64::new(0));
    let weak = ui.as_weak();
    ui.on_autostart_requested(move |enabled| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_autostart_busy() {
            return;
        }
        ui.set_autostart_busy(true);
        let current = revision.fetch_add(1, Ordering::AcqRel) + 1;
        let revision = Arc::clone(&revision);
        let weak = weak.clone();
        runtime.spawn_blocking(move || {
            let directory = crate::application::platform::config_dir();
            let result = crate::autostart::set_enabled(directory.clone(), enabled);
            let actual = crate::autostart::enabled(directory);
            let _ = slint::invoke_from_event_loop(move || {
                if revision.load(Ordering::Acquire) != current {
                    return;
                }
                let Some(ui) = weak.upgrade() else { return };
                ui.set_autostart_busy(false);
                match actual {
                    Ok(actual) => ui.set_autostart_enabled(actual),
                    Err(error) => {
                        ui.set_autostart_message(error.message.into());
                        return;
                    }
                }
                ui.set_autostart_message(match result {
                    Ok(()) if enabled => "Sillage will start in the background at login.".into(),
                    Ok(()) => "Sillage will not start automatically at login.".into(),
                    Err(error) => error.message.into(),
                });
            });
        });
    });
}
