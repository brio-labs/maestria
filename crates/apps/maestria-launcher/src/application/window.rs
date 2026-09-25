use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::{ComponentHandle, SharedString};

use super::search::start_search;
use super::{CATALOG_REFRESH_TICKS, Frontend, UiWeak, lock};
use crate::LauncherWindow;
use crate::ipc::LauncherState;
use crate::model::ShortcutStatus;
use crate::shortcuts::Shortcuts;
pub(super) fn activate_launcher(
    window: &LauncherWindow,
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
) {
    match state.request_activation() {
        Ok(Some(generation)) => {
            frontend.generation.store(generation, Ordering::Release);
            {
                let mut model = lock(&frontend.model);
                model.query.clear();
                model.pending_ticks = None;
                model.accepted.clear();
                model.accepted_passages.clear();
                model.accepted_paths.clear();
                model.displayed.clear();
                model.selected_file = None;
                model.content_view_passages.clear();
                model.result_filter = "all".to_string();
            }
            window.set_query(SharedString::default());
            window.set_preferences_open(false);
            window.set_reset_confirmation(false);
            window.set_about_open(false);
            window.set_actions_open(false);
            window.set_notice(SharedString::default());
            window.set_passage_view_open(false);
            window.set_passage_view_results(super::empty_results());
            window.set_result_filter("all".into());
            let _ = window.window().show();
            #[cfg(target_os = "linux")]
            {
                use slint::winit_030::WinitWindowAccessor;
                let _ = window
                    .window()
                    .with_winit_window(|native_window| native_window.focus_window());
            }
            window.invoke_focus_search();
            let _ = state.catalog().request_refresh();
            lock(&frontend.model).catalog_ticks_until_refresh = CATALOG_REFRESH_TICKS;
            start_search(state, frontend, runtime, window.as_weak(), String::new());
        }
        Ok(None) => {}
        Err(error) => show_notice(&window.as_weak(), error.message),
    }
}
pub(super) fn hide_launcher(ui: &UiWeak, state: &LauncherState) {
    if let Err(error) = state.dismiss() {
        show_notice(ui, error.message);
        return;
    }
    if let Some(window) = ui.upgrade() {
        close_extension_panel(&window);
        if let Err(error) = window.window().hide() {
            show_notice(ui, error.to_string());
        }
    }
}
pub(super) fn close_extension_panel(window: &LauncherWindow) {
    if window.get_extensions_open() {
        window.set_extensions_open(false);
        window.invoke_extensions_closed();
    }
}

pub(super) fn quit_launcher(window: &LauncherWindow, state: &LauncherState, shortcuts: &Shortcuts) {
    if let Err(error) = state.allow_exit() {
        window.set_notice(error.message.into());
        return;
    }
    if let Err(error) = shortcuts.shutdown() {
        window.set_notice(error.message.into());
        return;
    }
    if let Err(error) = slint::quit_event_loop() {
        window.set_notice(error.to_string().into());
    }
}
pub(super) fn shortcut_label(status: &ShortcutStatus) -> String {
    match status.message.as_deref() {
        Some(message) if !message.is_empty() => format!("{} — {message}", status.description),
        _ => status.description.clone(),
    }
}
pub(super) fn show_notice(ui: &UiWeak, message: String) {
    let ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_notice(message.into());
            if window.get_preferences_open() {
                window.set_preferences_warning(window.get_notice());
            } else {
                window.set_status_message(window.get_notice());
                window.set_status_kind("error".into());
            }
        }
    });
}
