use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine;
use gio::prelude::*;
use gtk::prelude::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::catalog::AppEntry;
use crate::errors::LauncherError;
use crate::model::{ResultKind, SearchResult};

const APPLICATION_ID_PREFIX: &str = "app:";
const ICON_SIZE: i32 = 28;
const MAX_ICON_LOGICAL_SIZE: i32 = 64;
const MAX_ICON_CACHE_ENTRIES: usize = 128;
const MAX_ICON_DATA_URI_BYTES: usize = 128 * 1024;
const PNG_DATA_URI_PREFIX: &str = "data:image/png;base64,";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayBackend {
    X11,
    Wayland,
    Unavailable,
}

struct MonitorRegistration {
    monitor: gio::AppInfoMonitor,
    handler_id: Option<glib::SignalHandlerId>,
}

impl Drop for MonitorRegistration {
    fn drop(&mut self) {
        if let Some(handler_id) = self.handler_id.take() {
            self.monitor.disconnect(handler_id);
        }
    }
}

type IconCache = BTreeMap<String, (i32, Option<String>)>;

thread_local! {
    static APP_INFO_MONITOR: RefCell<Option<MonitorRegistration>> = const { RefCell::new(None) };
    static ICON_CACHE: RefCell<IconCache> = const { RefCell::new(BTreeMap::new()) };
}

/// Enumerate the desktop applications visible to the current desktop.
///
/// This function intentionally does not touch GTK. The caller can run it on a
/// catalog worker: all GIO objects are created and dropped on that worker, and
/// only owned application data is returned.
pub fn enumerate_apps() -> Result<Vec<AppEntry>, LauncherError> {
    let mut by_id = BTreeMap::new();

    for app_info in gio::AppInfo::all() {
        let Ok(candidate) = app_info.downcast::<gio::DesktopAppInfo>() else {
            continue;
        };
        let Some(candidate_id) = candidate
            .id()
            .and_then(|value| owned_string(value.as_str()))
        else {
            continue;
        };
        if candidate_id.is_empty() {
            continue;
        }
        // Resolve the ID again through GIO so user/system desktop-entry
        // precedence is honored even if a backend returns duplicate entries.
        let Some(app_info) = gio::DesktopAppInfo::new(&candidate_id) else {
            continue;
        };

        if !is_eligible(&app_info) {
            continue;
        }

        let Some(desktop_id) = app_info.id().and_then(|value| owned_string(value.as_str())) else {
            continue;
        };
        if desktop_id.is_empty() {
            continue;
        }

        let display_name = app_info.display_name();
        let Some(name) = owned_string(display_name.as_str()) else {
            continue;
        };

        let description = match app_info.description() {
            Some(value) => {
                let Some(value) = owned_string(value.as_str()) else {
                    continue;
                };
                value
            }
            None => String::new(),
        };

        let mut keywords = Vec::new();
        let mut invalid_keyword = false;
        for keyword in app_info.keywords() {
            let Some(keyword) = owned_string(keyword.as_str()) else {
                invalid_keyword = true;
                break;
            };
            keywords.push(keyword);
        }
        if invalid_keyword {
            continue;
        }

        let icon_descriptor = match app_info.icon() {
            Some(icon) => match gio::prelude::IconExt::to_string(&icon) {
                Some(value) => {
                    let Some(value) = owned_string(value.as_str()) else {
                        continue;
                    };
                    Some(value)
                }
                None => None,
            },
            None => None,
        };

        let Some(desktop_file) = app_info.filename().and_then(|path| path_to_string(&path)) else {
            continue;
        };

        by_id.entry(desktop_id.clone()).or_insert_with(|| {
            AppEntry::new(
                desktop_id,
                name,
                description,
                keywords,
                icon_descriptor,
                desktop_file,
            )
        });
    }

    Ok(by_id.into_values().collect())
}

/// Launch a desktop application through GIO. This must run on the GTK main
/// thread because it creates a display launch context and may touch the native
/// window/display.
pub fn launch_app(window: &tauri::WebviewWindow, desktop_id: &str) -> Result<(), LauncherError> {
    require_main_thread()?;
    if desktop_id.is_empty() || desktop_id.as_bytes().contains(&0) {
        return Err(LauncherError::app_unavailable(
            "the selected application identifier is invalid",
        ));
    }

    let Some(app_info) = gio::DesktopAppInfo::new(desktop_id) else {
        return Err(LauncherError::app_unavailable(
            "the selected application is no longer installed",
        ));
    };
    if app_info.id().as_ref().map(|value| value.as_str()) != Some(desktop_id)
        || !is_eligible(&app_info)
    {
        return Err(LauncherError::app_unavailable(
            "the selected application is no longer available",
        ));
    }
    let Some(desktop_file) = app_info.filename() else {
        return Err(LauncherError::app_unavailable(
            "the selected application has no desktop file",
        ));
    };
    if !desktop_file.is_file() {
        return Err(LauncherError::app_unavailable(
            "the selected application desktop file was removed",
        ));
    }

    let native_window = window
        .gtk_window()
        .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?;
    let display = native_window.display();
    let Some(launch_context) = display.app_launch_context() else {
        return Err(LauncherError::platform_unavailable(
            "the native display cannot create an application launch context",
        ));
    };

    app_info
        .launch(&[], Some(&launch_context))
        .map_err(|error| LauncherError::launch_failed(error.to_string()))
}

/// Fill application result rows with bounded, local PNG data URIs. Icon/theme
/// APIs are GTK main-thread APIs; an unavailable theme or a decode/encode
/// failure simply leaves the row without an icon.
pub fn decorate_results(
    window: &tauri::WebviewWindow,
    apps: &[AppEntry],
    results: &mut [SearchResult],
) {
    if !gtk::is_initialized_main_thread() {
        return;
    }

    let Ok(native_window) = window.gtk_window() else {
        return;
    };
    let scale = native_window.scale_factor().max(1);
    let Some(icon_theme) = gtk::IconTheme::default() else {
        for result in results {
            if matches!(&result.kind, ResultKind::Application) {
                result.icon = None;
            }
        }
        return;
    };

    for result in results {
        if !matches!(&result.kind, ResultKind::Application) {
            continue;
        }

        let Some(desktop_id) = result.id.strip_prefix(APPLICATION_ID_PREFIX) else {
            result.icon = None;
            continue;
        };
        let Some(app) = apps.iter().find(|app| app.desktop_id == desktop_id) else {
            result.icon = None;
            continue;
        };
        let Some(descriptor) = app.icon_descriptor.as_deref() else {
            result.icon = None;
            continue;
        };

        let icon = ICON_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if let Some((cached_scale, value)) = cache.get(descriptor)
                && *cached_scale == scale
            {
                return value.clone();
            }

            let value = rasterize_icon(&icon_theme, descriptor, scale);
            if cache.len() >= MAX_ICON_CACHE_ENTRIES {
                cache.pop_first();
            }
            cache.insert(descriptor.to_string(), (scale, value.clone()));
            value
        });
        result.icon = icon;
    }
}

/// Install (or replace) the process-local GIO application monitor. This must
/// be called on the GTK main thread so the monitor and its signal stay owned by
/// the resident main context.
pub fn install_monitor<F>(on_changed: F) -> Result<(), LauncherError>
where
    F: Fn() + 'static,
{
    require_main_thread()?;
    let monitor = gio::AppInfoMonitor::get();
    let handler_id = monitor.connect_changed(move |_| {
        ICON_CACHE.with(|cache| cache.borrow_mut().clear());
        on_changed();
    });
    APP_INFO_MONITOR.with(|slot| {
        *slot.borrow_mut() = Some(MonitorRegistration {
            monitor,
            handler_id: Some(handler_id),
        });
    });
    Ok(())
}

/// Determine the active backend from the native handles Tauri gives us. No
/// DISPLAY/WAYLAND_DISPLAY environment heuristic is used.
pub fn display_backend(window: &tauri::WebviewWindow) -> DisplayBackend {
    if !gtk::is_initialized_main_thread() {
        return DisplayBackend::Unavailable;
    }
    let Ok(native) = window.gtk_window() else {
        return DisplayBackend::Unavailable;
    };
    // Tao queues visibility changes; show() does not guarantee realization yet.
    native.realize();
    let Ok(window_handle) = window.window_handle() else {
        return DisplayBackend::Unavailable;
    };
    // Inspect the actual GTK display. Tao's X11 raw-display accessor opens a new
    // X connection, so it must not be used as a recurring backend probe.
    match (native.display().type_().name(), window_handle.as_raw()) {
        ("GdkX11Display", RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)) => {
            DisplayBackend::X11
        }
        ("GdkWaylandDisplay", RawWindowHandle::Wayland(_)) => DisplayBackend::Wayland,
        _ => DisplayBackend::Unavailable,
    }
}

/// Apply a portal activation token through the GTK3 startup-ID path. The
/// token is only accepted as a NUL-free value and is never interpreted.
pub fn apply_activation_token(
    window: &tauri::WebviewWindow,
    token: &str,
) -> Result<(), LauncherError> {
    require_main_thread()?;
    if token.is_empty() || token.as_bytes().contains(&0) {
        return Err(LauncherError::invalid_request(
            "the activation token is invalid",
        ));
    }

    let native_window = window
        .gtk_window()
        .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?;
    let Some(gdk_window) = native_window.window() else {
        return Err(LauncherError::platform_unavailable(
            "the launcher native window is not realized",
        ));
    };
    gdk_window.set_startup_id(token);
    Ok(())
}

fn require_main_thread() -> Result<(), LauncherError> {
    if gtk::is_initialized_main_thread() {
        Ok(())
    } else {
        Err(LauncherError::platform_unavailable(
            "this Linux platform operation must run on the GTK main thread",
        ))
    }
}

fn is_eligible(app_info: &gio::DesktopAppInfo) -> bool {
    app_info.should_show()
        && !app_info.is_hidden()
        && !app_info.is_nodisplay()
        && app_info.shows_in(None)
}

fn owned_string(value: &str) -> Option<String> {
    if value.as_bytes().contains(&0) {
        None
    } else {
        Some(value.to_string())
    }
}

fn path_to_string(path: &Path) -> Option<String> {
    owned_string(path.to_str()?)
}

fn rasterize_icon(theme: &gtk::IconTheme, descriptor: &str, scale: i32) -> Option<String> {
    // The renderer receives only a data URI. Theme names and local file-icon
    // descriptors are resolved by GTK; remote URI schemes and raw paths are
    // never treated as renderer-visible URLs.
    if descriptor.is_empty() || descriptor.as_bytes().contains(&0) {
        return None;
    }
    let icon = gio::Icon::for_string(descriptor).ok()?;
    if let Some(file_icon) = icon.downcast_ref::<gio::FileIcon>() {
        // File icons must resolve to local paths; never ask GVfs to fetch a URI.
        file_icon.file().path()?;
    } else if !icon.is::<gio::ThemedIcon>() {
        return None;
    }
    let requested_scale = scale.max(1);

    let icon_info = theme.lookup_by_gicon_for_scale(
        &icon,
        ICON_SIZE,
        requested_scale,
        gtk::IconLookupFlags::FORCE_SIZE,
    )?;
    let pixbuf = icon_info.load_icon().ok()?;

    let max_size = MAX_ICON_LOGICAL_SIZE.saturating_mul(requested_scale);
    let width = pixbuf.width();
    let height = pixbuf.height();
    if width <= 0 || height <= 0 || max_size <= 0 {
        return None;
    }

    let pixbuf = if width > max_size || height > max_size {
        let largest = i64::from(width.max(height));
        let target_width = ((i64::from(width) * i64::from(max_size)) / largest)
            .max(1)
            .min(i64::from(i32::MAX)) as i32;
        let target_height = ((i64::from(height) * i64::from(max_size)) / largest)
            .max(1)
            .min(i64::from(i32::MAX)) as i32;
        pixbuf.scale_simple(
            target_width,
            target_height,
            gdk_pixbuf::InterpType::Bilinear,
        )?
    } else {
        pixbuf
    };

    let png = pixbuf.save_to_bufferv("png", &[]).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    if PNG_DATA_URI_PREFIX.len() + encoded.len() > MAX_ICON_DATA_URI_BYTES {
        return None;
    }
    Some(format!("{PNG_DATA_URI_PREFIX}{encoded}"))
}
