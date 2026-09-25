use std::error::Error;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};

#[cfg(target_os = "linux")]
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Timer, TimerMode};

use super::platform::{config_dir, system_dark_mode};
use super::preferences::{configure_saved_shortcut, sync_preferences};
use super::search::start_search;
use super::window::{activate_launcher, quit_launcher, shortcut_label};
use super::{
    CATALOG_REFRESH_TICKS, Frontend, FrontendModel, LauncherWindow, RuntimeMessage, UI_TICK,
    UiWeak, has_argument, lock,
};
use crate::ipc::LauncherState;
use crate::shortcuts::Shortcuts;

struct TimerState {
    initialized: bool,
    last_catalog_revision: u64,
    last_shortcut_label: String,
}

struct TimerContext {
    ui_weak: UiWeak,
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    shortcuts: Arc<Shortcuts>,
    runtime: tokio::runtime::Handle,
    control_receiver: mpsc::Receiver<RuntimeMessage>,
    activation_receiver: mpsc::Receiver<()>,
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    if has_argument(&arguments, "--quit") {
        let _ = super::instance::send_existing(RuntimeMessage::Quit)?;
        return Ok(());
    }

    let (control_sender, control_receiver) = mpsc::sync_channel(16);
    let Some(instance) = super::instance::PrimaryInstance::claim(control_sender)? else {
        return Ok(());
    };
    run_primary(instance, control_receiver)
}

fn run_primary(
    instance: super::instance::PrimaryInstance,
    control_receiver: mpsc::Receiver<RuntimeMessage>,
) -> Result<(), Box<dyn Error>> {
    select_backend()?;

    let state = Arc::new(crate::ipc::LauncherState::new(
        crate::settings::SettingsManager::load(config_dir()),
    )?);
    state.catalog().request_refresh()?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("maestria-launcher-async")
        .enable_all()
        .build()?;
    let runtime_handle = runtime.handle().clone();
    let (activation_sender, activation_receiver) = mpsc::sync_channel(8);
    let shortcuts = Arc::new(Shortcuts::new(activation_sender));
    let frontend = Arc::new(Frontend {
        generation: std::sync::atomic::AtomicU64::new(0),
        model: std::sync::Mutex::new(FrontendModel {
            query: String::new(),
            pending_ticks: None,
            accepted: Vec::new(),
            selected_file: None,
            catalog_ticks_until_refresh: CATALOG_REFRESH_TICKS,
            accepted_passages: Vec::new(),
            accepted_paths: Vec::new(),
            displayed: Vec::new(),
            result_filter: "all".to_string(),
            content_view_passages: Vec::new(),
        }),
    });

    let ui = LauncherWindow::new()?;
    let system_dark = system_dark_mode();
    initialize_ui(&ui, &state, &shortcuts, system_dark)?;
    super::callbacks::install_callbacks(
        &ui,
        Arc::clone(&state),
        Arc::clone(&frontend),
        Arc::clone(&shortcuts),
        runtime_handle.clone(),
        system_dark,
    );
    super::extensions::install_callbacks(&ui, Arc::clone(&state), runtime_handle.clone());
    install_window_handlers(&ui, Arc::clone(&state));
    let _timer = install_timer(
        ui.as_weak(),
        Arc::clone(&state),
        Arc::clone(&frontend),
        Arc::clone(&shortcuts),
        runtime_handle,
        control_receiver,
        activation_receiver,
    );

    ui.window().show()?;
    // Hiding the only window must not terminate the resident singleton.
    slint::run_event_loop_until_quit()?;
    let _ = shortcuts.shutdown();
    drop(instance);
    drop(runtime);
    Ok(())
}

fn select_backend() -> Result<(), Box<dyn Error>> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_window_attributes_hook(|attributes| attributes.with_decorations(false))
        .select()?;
    Ok(())
}

fn initialize_ui(
    ui: &LauncherWindow,
    state: &LauncherState,
    shortcuts: &Shortcuts,
    system_dark: bool,
) -> Result<(), crate::errors::LauncherError> {
    ui.set_system_dark_mode(system_dark);
    ui.set_dark_mode(system_dark);
    ui.set_status_kind("loading".into());
    ui.set_status_message("Discovering installed applications…".into());
    sync_preferences(ui, state, shortcuts, system_dark)
}

fn install_window_handlers(ui: &LauncherWindow, state: Arc<LauncherState>) {
    let close_state = Arc::clone(&state);
    let close_ui = ui.as_weak();
    ui.window().on_close_requested(move || {
        if close_state
            .hide_requested()
            .is_ok_and(|requested| requested)
            && close_state.dismiss().is_ok()
        {
            if let Some(window) = close_ui.upgrade() {
                super::window::close_extension_panel(&window);
            }
            slint::CloseRequestResponse::HideWindow
        } else {
            slint::CloseRequestResponse::KeepWindowShown
        }
    });

    #[cfg(target_os = "linux")]
    {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit};

        let mut was_focused = false;
        let blur_ui = ui.as_weak();
        ui.window().on_winit_window_event(move |window, event| {
            match event {
                winit::event::WindowEvent::Focused(true) => was_focused = true,
                winit::event::WindowEvent::Focused(false)
                    if was_focused && state.hide_requested().is_ok_and(|requested| requested) =>
                {
                    was_focused = false;
                    match state.dismiss() {
                        Ok(()) => {
                            if let Some(window_ui) = blur_ui.upgrade() {
                                super::window::close_extension_panel(&window_ui);
                            }
                            if let Err(error) = window.hide() {
                                eprintln!("Launcher dismissal failed: {error}");
                            }
                        }
                        Err(error) => {
                            if let Some(window_ui) = blur_ui.upgrade() {
                                window_ui.set_notice(error.message.into());
                            }
                        }
                    }
                }
                _ => {}
            }
            EventResult::Propagate
        });
    }
}

fn install_timer(
    ui: UiWeak,
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    shortcuts: Arc<Shortcuts>,
    runtime: tokio::runtime::Handle,
    control_receiver: mpsc::Receiver<RuntimeMessage>,
    activation_receiver: mpsc::Receiver<()>,
) -> Timer {
    let mut timer_state = TimerState {
        initialized: false,
        last_catalog_revision: 0,
        last_shortcut_label: String::new(),
    };
    let context = TimerContext {
        ui_weak: ui,
        state,
        frontend,
        shortcuts,
        runtime,
        control_receiver,
        activation_receiver,
    };
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, UI_TICK, move || {
        on_timer_tick(&mut timer_state, &context);
    });
    timer
}

fn on_timer_tick(timer: &mut TimerState, context: &TimerContext) {
    let Some(ui) = context.ui_weak.upgrade() else {
        return;
    };

    initialize_when_ready(
        timer,
        &ui,
        &context.state,
        &context.frontend,
        &context.shortcuts,
        &context.runtime,
    );
    handle_runtime_messages(
        &ui,
        &context.state,
        &context.frontend,
        &context.shortcuts,
        &context.runtime,
        &context.control_receiver,
    );
    handle_shortcut_activation(
        &ui,
        &context.state,
        &context.frontend,
        &context.runtime,
        &context.activation_receiver,
    );

    if let Some(query) = take_debounced_query(&context.frontend) {
        start_search(
            Arc::clone(&context.state),
            Arc::clone(&context.frontend),
            context.runtime.clone(),
            ui.as_weak(),
            query,
        );
    }

    if timer.initialized {
        refresh_catalog_if_due(&ui, &context.state, &context.frontend);
        update_catalog_status(
            timer,
            &ui,
            Arc::clone(&context.state),
            Arc::clone(&context.frontend),
            &context.runtime,
        );
        update_shortcut_status(timer, &ui, &context.shortcuts);
    }
}

fn initialize_when_ready(
    timer: &mut TimerState,
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    frontend: &Arc<Frontend>,
    shortcuts: &Arc<Shortcuts>,
    runtime: &tokio::runtime::Handle,
) {
    if timer.initialized {
        return;
    }
    #[cfg(target_os = "linux")]
    let window_ready = ui.window().has_winit_window();
    #[cfg(not(target_os = "linux"))]
    let window_ready = true;
    if !window_ready {
        return;
    }

    timer.initialized = true;
    if let Err(error) = shortcuts.initialize(ui.window(), runtime) {
        ui.set_shortcut_status(error.message.into());
    }
    match state.mark_ready() {
        Ok(generation) => {
            frontend.generation.store(generation, Ordering::Release);
            ui.invoke_focus_search();
            let query = lock(&frontend.model).query.clone();
            start_search(
                Arc::clone(state),
                Arc::clone(frontend),
                runtime.clone(),
                ui.as_weak(),
                query,
            );
            configure_saved_shortcut(
                Arc::clone(state),
                Arc::clone(shortcuts),
                runtime.clone(),
                ui.as_weak(),
            );
        }
        Err(error) => ui.set_status_message(error.message.into()),
    }
}

fn handle_runtime_messages(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    frontend: &Arc<Frontend>,
    shortcuts: &Arc<Shortcuts>,
    runtime: &tokio::runtime::Handle,
    receiver: &mpsc::Receiver<RuntimeMessage>,
) {
    while let Ok(message) = receiver.try_recv() {
        match message {
            RuntimeMessage::Activate => {
                activate_launcher(ui, Arc::clone(state), Arc::clone(frontend), runtime.clone())
            }
            RuntimeMessage::Quit => quit_launcher(ui, state, shortcuts),
        }
    }
}

fn handle_shortcut_activation(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    frontend: &Arc<Frontend>,
    runtime: &tokio::runtime::Handle,
    receiver: &mpsc::Receiver<()>,
) {
    if receiver.try_recv().is_err() {
        return;
    }
    while receiver.try_recv().is_ok() {}
    activate_launcher(ui, Arc::clone(state), Arc::clone(frontend), runtime.clone());
}

fn take_debounced_query(frontend: &Frontend) -> Option<String> {
    let mut model = lock(&frontend.model);
    match model.pending_ticks {
        Some(1) => {
            model.pending_ticks = None;
            Some(model.query.clone())
        }
        Some(remaining) => {
            model.pending_ticks = Some(remaining.saturating_sub(1));
            None
        }
        None => None,
    }
}

fn refresh_catalog_if_due(ui: &LauncherWindow, state: &LauncherState, frontend: &Frontend) {
    let due = {
        let mut model = lock(&frontend.model);
        model.catalog_ticks_until_refresh = model.catalog_ticks_until_refresh.saturating_sub(1);
        model.catalog_ticks_until_refresh == 0
    };
    if due && ui.window().is_visible() && state.catalog().request_refresh().is_ok() {
        lock(&frontend.model).catalog_ticks_until_refresh = CATALOG_REFRESH_TICKS;
    }
}

fn update_catalog_status(
    timer: &mut TimerState,
    ui: &LauncherWindow,
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: &tokio::runtime::Handle,
) {
    let Ok(snapshot) = state.catalog().snapshot() else {
        return;
    };
    let ready = matches!(
        &snapshot.status.kind,
        crate::model::SearchStatusKind::Ready | crate::model::SearchStatusKind::Error
    );
    if ready && snapshot.revision != timer.last_catalog_revision {
        timer.last_catalog_revision = snapshot.revision;
        let query = lock(&frontend.model).query.clone();
        start_search(state, frontend, runtime.clone(), ui.as_weak(), query);
    } else if matches!(
        &snapshot.status.kind,
        crate::model::SearchStatusKind::Loading | crate::model::SearchStatusKind::Refreshing
    ) {
        ui.set_status_kind("loading".into());
        let message = snapshot.status.message.as_deref().map_or_else(
            || "Refreshing installed applications…".to_string(),
            str::to_owned,
        );
        ui.set_status_message(message.into());
    }
}

fn update_shortcut_status(timer: &mut TimerState, ui: &LauncherWindow, shortcuts: &Shortcuts) {
    let label = shortcut_label(&shortcuts.status());
    if label != timer.last_shortcut_label {
        ui.set_shortcut_status(label.clone().into());
        timer.last_shortcut_label = label;
    }
}
