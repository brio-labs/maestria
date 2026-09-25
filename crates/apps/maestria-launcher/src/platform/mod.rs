#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{
    DisplayBackend, detect_display_backend, enumerate_apps, launch_app, open_local_file,
    open_local_pdf_page, window_identifier,
};

#[cfg(not(target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayBackend {
    X11,
    Wayland,
    Unavailable,
}

#[cfg(not(target_os = "linux"))]
pub fn enumerate_apps() -> Result<Vec<crate::catalog::AppEntry>, crate::errors::LauncherError> {
    Err(crate::errors::LauncherError::platform_unavailable(
        "application discovery is unavailable on this platform",
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn launch_app(_desktop_id: &str) -> Result<(), crate::errors::LauncherError> {
    Err(crate::errors::LauncherError::platform_unavailable(
        "application launching is unavailable on this platform",
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn open_local_file(_path: &std::path::Path) -> Result<(), crate::errors::LauncherError> {
    Err(crate::errors::LauncherError::platform_unavailable(
        "opening local files is unavailable on this platform",
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn open_local_pdf_page(
    path: &std::path::Path,
    _page: u32,
) -> Result<(), crate::errors::LauncherError> {
    open_local_file(path)
}

#[cfg(not(target_os = "linux"))]
pub fn detect_display_backend(
    _window: &slint::Window,
) -> Result<DisplayBackend, crate::errors::LauncherError> {
    Ok(DisplayBackend::Unavailable)
}
