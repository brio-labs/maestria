use slint::ComponentHandle;
use std::sync::Arc;

use super::dispatch::{DispatchContext, dispatch_action};
use super::preferences::{
    clear_shortcut, configure_shortcut, defer_shortcut_setup, save_preferences,
};
use super::search::{
    apply_result_filter, close_passage_view, open_passage_view, passage_result_is_visible,
    update_selected_actions,
};
use super::window::{hide_launcher, show_notice};
use super::{Frontend, LauncherWindow, SEARCH_DEBOUNCE_TICKS, empty_actions, empty_results, lock};
use crate::ipc::LauncherState;
use crate::model::{
    HOST_COPY_ACTIVATION, HOST_PREFERENCES, HOST_RESET_PREFERENCES, PreferencesUpdate,
};
use crate::shortcuts::Shortcuts;

pub(super) fn install_callbacks(
    ui: &LauncherWindow,
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    shortcuts: Arc<Shortcuts>,
    runtime: tokio::runtime::Handle,
    system_dark: bool,
) {
    install_query_callbacks(ui, &frontend);
    install_result_callback(ui, &state, &frontend, &shortcuts, &runtime, system_dark);
    install_action_callback(ui, &state, &frontend, &shortcuts, &runtime, system_dark);
    install_preferences_callbacks(ui, &state, &shortcuts, system_dark);
    install_shortcut_callbacks(ui, &state, &shortcuts, &runtime, system_dark);
    install_host_action_callbacks(ui, &state, &frontend, &shortcuts, &runtime, system_dark);
    install_save_callback(ui, &state, &shortcuts, &runtime, system_dark);
    install_dismiss_callback(ui, &state);
    install_search_surface_callbacks(ui, &frontend);
}

fn install_query_callbacks(ui: &LauncherWindow, frontend: &Arc<Frontend>) {
    let weak = ui.as_weak();
    let query_frontend = Arc::clone(frontend);
    ui.on_query_changed(move |query| {
        let mut model = lock(&query_frontend.model);
        model.query = query.to_string();
        model.pending_ticks = Some(SEARCH_DEBOUNCE_TICKS);
        model.accepted.clear();
        model.accepted_passages.clear();
        model.accepted_paths.clear();
        model.displayed.clear();
        model.content_view_passages.clear();
        model.result_filter = "all".to_string();
        model.selected_file = None;
        query_frontend
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        drop(model);
        if let Some(ui) = weak.upgrade() {
            ui.set_results(empty_results());
            ui.set_actions(empty_actions());
            ui.set_actions_open(false);
            ui.set_selected_action_index(0);
            ui.set_selected_index(0);
            ui.set_passage_view_open(false);
            ui.set_passage_view_results(empty_results());
            ui.set_result_filter("all".into());
            ui.set_index_status("Index status pending".into());
            ui.set_status_kind("loading".into());
            ui.set_status_message("Searching…".into());
        }
    });

    let weak = ui.as_weak();
    let selection_frontend = Arc::clone(frontend);
    ui.on_selection_changed(move |index| {
        if let Some(ui) = weak.upgrade() {
            update_selected_actions(&ui, &selection_frontend, index.max(0) as usize);
        }
    });
}

fn install_search_surface_callbacks(ui: &LauncherWindow, frontend: &Arc<Frontend>) {
    let weak = ui.as_weak();
    let filter_frontend = Arc::clone(frontend);
    ui.on_result_filter_changed(move |filter| {
        if let Some(window) = weak.upgrade() {
            apply_result_filter(&window, &filter_frontend, filter.as_str());
        }
    });

    let weak = ui.as_weak();
    let view_frontend = Arc::clone(frontend);
    ui.on_passage_view_requested(move |result_id| {
        if let Some(window) = weak.upgrade() {
            open_passage_view(&window, &view_frontend, result_id.as_str());
        }
    });

    let weak = ui.as_weak();
    let close_frontend = Arc::clone(frontend);
    ui.on_passage_view_closed(move || {
        if let Some(window) = weak.upgrade() {
            close_passage_view(&window, &close_frontend);
        }
    });
}

fn install_dismiss_callback(ui: &LauncherWindow, state: &Arc<LauncherState>) {
    let weak = ui.as_weak();
    let dismiss_state = Arc::clone(state);
    ui.on_dismissed(move || hide_launcher(&weak, &dismiss_state));
}

fn install_result_callback(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    frontend: &Arc<Frontend>,
    shortcuts: &Arc<Shortcuts>,
    runtime: &tokio::runtime::Handle,
    system_dark: bool,
) {
    let weak = ui.as_weak();
    let action_state = Arc::clone(state);
    let action_frontend = Arc::clone(frontend);
    let action_shortcuts = Arc::clone(shortcuts);
    let action_runtime = runtime.clone();
    ui.on_result_activated(move |result_id| {
        let result_id = result_id.to_string();
        let action_id = {
            let model = lock(&action_frontend.model);
            if model
                .selected_file
                .as_ref()
                .is_some_and(|selected| selected.result_id == result_id)
            {
                Some("file.open".to_string())
            } else {
                model
                    .accepted
                    .iter()
                    .find(|result| result.id == result_id)
                    .and_then(super::search::primary_action)
                    .map(str::to_string)
            }
        };
        let Some(action_id) = action_id else {
            return;
        };
        dispatch_action(
            result_id,
            action_id,
            DispatchContext {
                state: Arc::clone(&action_state),
                frontend: Arc::clone(&action_frontend),
                shortcuts: Arc::clone(&action_shortcuts),
                runtime: action_runtime.clone(),
                ui: weak.clone(),
                system_dark,
            },
        );
    });
}

fn install_action_callback(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    frontend: &Arc<Frontend>,
    shortcuts: &Arc<Shortcuts>,
    runtime: &tokio::runtime::Handle,
    system_dark: bool,
) {
    let weak = ui.as_weak();
    let action_state = Arc::clone(state);
    let action_frontend = Arc::clone(frontend);
    let action_shortcuts = Arc::clone(shortcuts);
    let action_runtime = runtime.clone();
    ui.on_action_activated(move |result_id, action_id| {
        let result_id = result_id.to_string();
        let action_id = action_id.to_string();
        let path_action = super::search::path_result_is_visible(&action_frontend, &result_id);
        if !path_action && result_id.starts_with("path:") {
            return;
        }
        let passage_action = passage_result_is_visible(&action_frontend, &result_id);
        if !passage_action && result_id.starts_with("passage:") {
            return;
        }
        if path_action {
            super::search::activate_path_action(
                Arc::clone(&action_state),
                Arc::clone(&action_frontend),
                action_runtime.clone(),
                weak.clone(),
                result_id,
                action_id,
            );
        } else if passage_action {
            super::search::activate_passage_action(
                Arc::clone(&action_state),
                Arc::clone(&action_frontend),
                action_runtime.clone(),
                weak.clone(),
                result_id,
                action_id,
            );
        } else {
            dispatch_action(
                result_id,
                action_id,
                DispatchContext {
                    state: Arc::clone(&action_state),
                    frontend: Arc::clone(&action_frontend),
                    shortcuts: Arc::clone(&action_shortcuts),
                    runtime: action_runtime.clone(),
                    ui: weak.clone(),
                    system_dark,
                },
            );
        }
    });
}

fn install_preferences_callbacks(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    shortcuts: &Arc<Shortcuts>,
    system_dark: bool,
) {
    let weak = ui.as_weak();
    let prefs_state = Arc::clone(state);
    let prefs_shortcuts = Arc::clone(shortcuts);
    ui.on_preferences_requested(move || {
        if let Err(error) = prefs_state.enter_preferences() {
            show_notice(&weak, error.message);
            return;
        }
        if let Some(ui) = weak.upgrade() {
            ui.set_preferences_open(true);
            if let Err(error) = super::preferences::sync_preferences(
                &ui,
                &prefs_state,
                &prefs_shortcuts,
                system_dark,
            ) {
                ui.set_preferences_warning(error.message.into());
            }
            ui.invoke_focus_preferences();
        }
    });

    let weak = ui.as_weak();
    let close_state = Arc::clone(state);
    ui.on_preferences_closed(move || {
        if let Err(error) = close_state.leave_preferences() {
            show_notice(&weak, error.message);
        }
        if let Some(ui) = weak.upgrade() {
            ui.set_reset_confirmation(false);
        }
    });
}

fn install_shortcut_callbacks(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    shortcuts: &Arc<Shortcuts>,
    runtime: &tokio::runtime::Handle,
    system_dark: bool,
) {
    let weak = ui.as_weak();
    let configure_state = Arc::clone(state);
    let configure_shortcuts = Arc::clone(shortcuts);
    let configure_runtime = runtime.clone();
    ui.on_shortcut_configure_requested(move || {
        if let Some(ui) = weak.upgrade() {
            let preferred = ui.get_shortcut().to_string();
            configure_shortcut(
                preferred,
                true,
                Arc::clone(&configure_state),
                Arc::clone(&configure_shortcuts),
                configure_runtime.clone(),
                weak.clone(),
                system_dark,
            );
        }
    });

    let weak = ui.as_weak();
    let clear_state = Arc::clone(state);
    let clear_shortcuts = Arc::clone(shortcuts);
    let clear_runtime = runtime.clone();
    ui.on_shortcut_clear_requested(move || {
        clear_shortcut(
            Arc::clone(&clear_state),
            Arc::clone(&clear_shortcuts),
            clear_runtime.clone(),
            weak.clone(),
            system_dark,
        );
    });
    let weak = ui.as_weak();
    let defer_state = Arc::clone(state);
    let defer_shortcuts = Arc::clone(shortcuts);
    ui.on_shortcut_offer_deferred(move || {
        defer_shortcut_setup(
            Arc::clone(&defer_state),
            Arc::clone(&defer_shortcuts),
            weak.clone(),
            system_dark,
        );
    });
}
fn install_host_action_callbacks(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    frontend: &Arc<Frontend>,
    shortcuts: &Arc<Shortcuts>,
    runtime: &tokio::runtime::Handle,
    system_dark: bool,
) {
    let weak = ui.as_weak();
    let copy_state = Arc::clone(state);
    let copy_frontend = Arc::clone(frontend);
    let copy_shortcuts = Arc::clone(shortcuts);
    let copy_runtime = runtime.clone();
    ui.on_copy_activation_requested(move || {
        dispatch_action(
            HOST_PREFERENCES.to_string(),
            HOST_COPY_ACTIVATION.to_string(),
            DispatchContext {
                state: Arc::clone(&copy_state),
                frontend: Arc::clone(&copy_frontend),
                shortcuts: Arc::clone(&copy_shortcuts),
                runtime: copy_runtime.clone(),
                ui: weak.clone(),
                system_dark,
            },
        );
    });

    let weak = ui.as_weak();
    let reset_state = Arc::clone(state);
    let reset_frontend = Arc::clone(frontend);
    let reset_shortcuts = Arc::clone(shortcuts);
    let reset_runtime = runtime.clone();
    ui.on_preferences_reset_requested(move |confirm| {
        if !confirm {
            if let Some(ui) = weak.upgrade() {
                ui.set_notice("Press Confirm reset to discard the saved preferences.".into());
            }
            return;
        }
        dispatch_action(
            HOST_PREFERENCES.to_string(),
            HOST_RESET_PREFERENCES.to_string(),
            DispatchContext {
                state: Arc::clone(&reset_state),
                frontend: Arc::clone(&reset_frontend),
                shortcuts: Arc::clone(&reset_shortcuts),
                runtime: reset_runtime.clone(),
                ui: weak.clone(),
                system_dark,
            },
        );
    });
}

fn install_save_callback(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    shortcuts: &Arc<Shortcuts>,
    runtime: &tokio::runtime::Handle,
    system_dark: bool,
) {
    let weak = ui.as_weak();
    let save_state = Arc::clone(state);
    let save_shortcuts = Arc::clone(shortcuts);
    let save_runtime = runtime.clone();
    ui.on_preferences_saved(move |shortcut, theme, reduce_motion| {
        save_preferences(
            PreferencesUpdate {
                shortcut: Some(shortcut.to_string()),
                theme: Some(theme.to_string()),
                reduce_motion: Some(reduce_motion),
                shortcut_setup: None,
                confirm_reset: None,
            },
            Arc::clone(&save_state),
            Arc::clone(&save_shortcuts),
            save_runtime.clone(),
            weak.clone(),
            system_dark,
        );
    });
}
