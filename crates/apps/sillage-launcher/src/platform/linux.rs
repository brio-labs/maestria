use std::collections::BTreeMap;
use std::path::Path;

use gio::prelude::*;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};

use crate::catalog::AppEntry;
use crate::errors::LauncherError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayBackend {
    X11,
    Wayland,
    Unavailable,
}

/// Enumerate desktop entries through GIO; only owned strings cross the worker boundary.
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

/// Launch a currently-installed desktop application with the native GIO context.
pub fn launch_app(desktop_id: &str) -> Result<(), LauncherError> {
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
    let context = gio::AppLaunchContext::new();
    app_info
        .launch(&[], Some(&context))
        .map_err(|error| LauncherError::launch_failed(error.to_string()))
}

pub fn open_local_file(path: &Path) -> Result<(), LauncherError> {
    open_local_file_uri(path, None)
}

/// Ask a default PDF handler to open a cited 1-based page using PDF Open Parameters.
pub fn open_local_pdf_page(path: &Path, page: u32) -> Result<(), LauncherError> {
    open_local_file_uri(path, Some(page))
}

fn open_local_file_uri(path: &Path, pdf_page: Option<u32>) -> Result<(), LauncherError> {
    if !path.is_absolute() || !path.is_file() {
        return Err(LauncherError::app_unavailable(
            "the selected file is no longer available",
        ));
    }
    let uri = gio::File::for_path(path).uri();
    match pdf_page.filter(|page| *page > 0) {
        Some(page) => {
            let uri = format!("{uri}#page={page}");
            gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        }
        None => gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>),
    }
    .map_err(|error| LauncherError::launch_failed(error.to_string()))
}

pub fn detect_display_backend(window: &slint::Window) -> Result<DisplayBackend, LauncherError> {
    let handles = window.window_handle();
    let window_handle = handles
        .window_handle()
        .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?
        .as_raw();
    let display_handle = handles
        .display_handle()
        .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?
        .as_raw();
    Ok(match (window_handle, display_handle) {
        (RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_), _) => DisplayBackend::X11,
        (RawWindowHandle::Wayland(_), _) => DisplayBackend::Wayland,
        _ => DisplayBackend::Unavailable,
    })
}
pub fn window_identifier(
    window: &slint::Window,
    runtime: &tokio::runtime::Handle,
) -> Option<ashpd::WindowIdentifier> {
    let handles = window.window_handle();
    let window_handle = handles.window_handle().ok()?.as_raw();
    let display_handle = handles.display_handle().ok()?.as_raw();
    runtime.block_on(ashpd::WindowIdentifier::from_raw_handle(
        &window_handle,
        Some(&display_handle),
    ))
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
