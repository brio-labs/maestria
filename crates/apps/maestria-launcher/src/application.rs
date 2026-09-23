use std::error::Error;

use serde::Serialize;

use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::errors::LauncherError;
use crate::ipc::LauncherState;
use crate::metrics::LauncherMetrics;
use crate::model::{LAUNCHER_WINDOW_LABEL, PreferencesDto};
use crate::settings::SettingsManager;
use crate::shortcuts::Shortcuts;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivationEvent {
    generation: u64,
    preferences: PreferencesDto,
    metrics_enabled: bool,
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let initial_quit = has_argument(&arguments, "--quit");
    let initial_activate = has_argument(&arguments, "--activate");
    let initial_settings = SettingsManager::load(Err("not initialized".to_string()));
    let launcher_state = LauncherState::new(initial_settings)?;
    let builder = build_builder(launcher_state, initial_quit, initial_activate);
    let app = builder.build(tauri::generate_context!())?;
    app.run(handle_run_event);
    Ok(())
}

fn build_builder(
    launcher_state: LauncherState,
    initial_quit: bool,
    initial_activate: bool,
) -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default()
        // This must remain the first plugin: secondary invocations are routed before
        // any other plugin can initialize a second native resource.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Err(error) = secondary_invocation(app, &args) {
                eprintln!("Launcher invocation failed: {error}");
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(launcher_state)
        .manage(Shortcuts::new())
        .manage(LauncherMetrics::new())
        .invoke_handler(tauri::generate_handler![
            crate::ipc::launcher_ready,
            crate::ipc::search,
            crate::ipc::execute_action,
            crate::ipc::dismiss,
            crate::ipc::get_preferences,
            crate::ipc::save_preferences,
            crate::ipc::configure_shortcut,
            crate::ipc::frame_ready
        ])
        .on_window_event(|window, event| {
            if window.label() != LAUNCHER_WINDOW_LABEL {
                return;
            }
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    if let Err(error) = hide_window(window) {
                        eprintln!("Launcher dismissal failed: {error}");
                    }
                }
                WindowEvent::Focused(false) => {
                    if let Err(error) = hide_window(window) {
                        eprintln!("Launcher dismissal failed: {error}");
                    }
                }
                _ => {}
            }
        })
        .setup(move |app| setup_application(app, initial_quit, initial_activate))
}

fn secondary_invocation(app: &AppHandle, args: &[String]) -> Result<(), LauncherError> {
    if has_argument(args, "--quit") {
        let state = app.try_state::<LauncherState>().ok_or_else(|| {
            LauncherError::platform_unavailable("Launcher lifecycle state is unavailable")
        })?;
        state.allow_exit()?;
        app.exit(0);
        Ok(())
    } else {
        present_existing(app)
    }
}

fn hide_window(window: &tauri::Window) -> Result<(), LauncherError> {
    let state = window.state::<LauncherState>();
    if !state.hide_requested()? {
        return Ok(());
    }
    state.dismiss()?;
    window
        .hide()
        .map_err(|error| LauncherError::platform_unavailable(error.to_string()))
}

fn setup_application(
    app: &mut tauri::App,
    initial_quit: bool,
    initial_activate: bool,
) -> Result<(), Box<dyn Error>> {
    let settings = SettingsManager::load(
        app.path()
            .app_config_dir()
            .map_err(|error| error.to_string()),
    );
    app.state::<LauncherState>().initialize_settings(settings)?;
    WebviewWindowBuilder::new(
        app,
        LAUNCHER_WINDOW_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title("Sillage Launcher")
    .inner_size(720.0, 480.0)
    .min_inner_size(480.0, 320.0)
    .prevent_overflow()
    .background_color(tauri::utils::config::Color(23, 26, 31, 255))
    .resizable(true)
    .decorations(false)
    .shadow(true)
    .visible(false)
    .on_navigation(|url| {
        (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
            || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"))
            || (cfg!(debug_assertions)
                && url.host_str() == Some("127.0.0.1")
                && url.port() == Some(1420))
    })
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .build()?;

    if initial_quit {
        let state = app.state::<LauncherState>();
        state.allow_exit()?;
        app.handle().exit(0);
    } else if initial_activate {
        // The window remains hidden until launcher_ready. The activation is
        // retained by request_activation and the ready command emits a fresh
        // generation after listeners have been installed.
        request_activation(app.handle())?;
    }
    if !initial_quit {
        let handle = app.handle().clone();
        let state = app.state::<LauncherState>();
        if let Err(error) =
            crate::platform::install_monitor(move || crate::catalog::changed(&handle))
        {
            state
                .catalog()
                .monitor_failed(error)
                .map_err(|error| std::io::Error::other(error.message))?;
        }
        state
            .catalog()
            .refresh_if_dirty(app.handle())
            .map_err(|error| std::io::Error::other(error.message))?;
    }
    Ok(())
}

fn handle_run_event(app: &AppHandle, event: RunEvent) {
    if let RunEvent::ExitRequested { ref api, .. } = event {
        let should_prevent = app
            .try_state::<LauncherState>()
            .is_none_or(|state| state.should_prevent_exit());
        if should_prevent {
            api.prevent_exit();
        } else if !crate::shortcuts::shutdown_complete(app) {
            match crate::shortcuts::request_shutdown(app) {
                Ok(_) => api.prevent_exit(),
                Err(error) => eprintln!("Shortcut shutdown could not start: {}", error.message),
            }
        }
    }
}

fn has_argument(arguments: &[String], wanted: &str) -> bool {
    arguments.iter().any(|argument| argument == wanted)
}

pub(crate) fn request_activation(app: &AppHandle) -> Result<Option<u64>, LauncherError> {
    let metrics = app.state::<LauncherMetrics>();
    let received_at = metrics.enabled().then(glib::monotonic_time);
    let generation = app.state::<LauncherState>().request_activation()?;
    if let Some(received_at) = received_at {
        metrics.activation_received(generation, received_at);
    }
    if let Some(generation) = generation {
        let handle = app.clone();
        app.run_on_main_thread(move || {
            if let Err(error) = present_generation(&handle, generation) {
                eprintln!("Launcher presentation failed: {}", error.message);
            }
        })
        .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?;
    }
    Ok(generation)
}

pub(crate) fn present_generation(app: &AppHandle, generation: u64) -> Result<(), LauncherError> {
    let window = app
        .get_webview_window(LAUNCHER_WINDOW_LABEL)
        .ok_or_else(|| LauncherError::platform_unavailable("Launcher window is unavailable"))?;
    window
        .show()
        .and_then(|()| window.set_focus())
        .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?;
    crate::shortcuts::initialize(app, &window)?;
    let state = app.state::<LauncherState>();
    let preferences = state.settings()?.dto(app.state::<Shortcuts>().status());
    app.emit_to(
        LAUNCHER_WINDOW_LABEL,
        "launcher://activate",
        ActivationEvent {
            generation,
            preferences,
            metrics_enabled: app.state::<LauncherMetrics>().enabled(),
        },
    )
    .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?;
    state.catalog().refresh_if_dirty(app)?;
    Ok(())
}

fn present_existing(app: &AppHandle) -> Result<(), LauncherError> {
    request_activation(app).map(|_| ())
}
