use std::fs::File;

use crate::errors::LauncherError;

#[derive(Debug, Clone, Copy)]
pub(super) enum DesktopError {
    NotFound,
    Unavailable,
    Failed,
}

pub(super) fn notify(title: &str, message: &str) -> Result<(), DesktopError> {
    #[cfg(target_os = "linux")]
    {
        notify_dbus(title, message)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (title, message);
        Err(DesktopError::Unavailable)
    }
}

#[cfg(target_os = "linux")]
fn notify_dbus(title: &str, message: &str) -> Result<(), DesktopError> {
    use std::collections::BTreeMap;

    use glib::variant::ToVariant;
    if title.contains('\0') || message.contains('\0') {
        return Err(DesktopError::Failed);
    }

    let address =
        std::env::var("DBUS_SESSION_BUS_ADDRESS").map_err(|_| DesktopError::Unavailable)?;
    let connection = gio::DBusConnection::for_address_sync(
        &address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        None::<&gio::Cancellable>,
    )
    .map_err(|_| DesktopError::Unavailable)?;
    let safe_title = glib::markup_escape_text(title);
    let safe_message = glib::markup_escape_text(message);
    let parameters = (
        "Sillage Launcher",
        0_u32,
        "",
        safe_title.as_str(),
        safe_message.as_str(),
        Vec::<String>::new(),
        BTreeMap::<String, glib::Variant>::new(),
        5_000_i32,
    )
        .to_variant();
    let response = connection
        .call_sync(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
            Some(&parameters),
            None,
            gio::DBusCallFlags::NONE,
            5_000,
            None::<&gio::Cancellable>,
        )
        .map_err(|_| DesktopError::Unavailable)?;
    if response.get::<(u32,)>().is_some() {
        Ok(())
    } else {
        Err(DesktopError::Failed)
    }
}

pub(super) fn open_url(url: &str) -> Result<(), DesktopError> {
    #[cfg(target_os = "linux")]
    {
        let parsed = url::Url::parse(url).map_err(|_| DesktopError::Failed)?;
        if parsed.scheme() != "https"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || local_host(parsed.host())
        {
            return Err(DesktopError::Failed);
        }
        gio::AppInfo::launch_default_for_uri(parsed.as_str(), None::<&gio::AppLaunchContext>)
            .map_err(|_| DesktopError::Unavailable)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = url;
        Err(DesktopError::Unavailable)
    }
}

#[cfg(target_os = "linux")]
fn local_host(host: Option<url::Host<&str>>) -> bool {
    match host {
        Some(url::Host::Domain(domain)) => {
            let name = domain.trim_end_matches('.').to_ascii_lowercase();
            name == "localhost"
                || name.ends_with(".localhost")
                || name.ends_with(".local")
                || name.ends_with(".internal")
        }
        Some(url::Host::Ipv4(address)) => {
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_multicast()
        }
        Some(url::Host::Ipv6(address)) => {
            address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_multicast()
        }
        None => true,
    }
}

pub(super) async fn open_selected_file(file: &File) -> Result<(), DesktopError> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsFd;

        let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let request = ashpd::desktop::open_uri::OpenFileRequest::default()
                .send_file(&file.as_fd())
                .await?;
            request.response()
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) | Err(_) => Err(DesktopError::Unavailable),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = file;
        Err(DesktopError::Unavailable)
    }
}

pub(super) fn copy_text(text: &str) -> Result<(), DesktopError> {
    crate::application::platform::copy_text(text).map_err(map_launcher_error)
}

fn map_launcher_error(error: LauncherError) -> DesktopError {
    match error.code.as_str() {
        "file_unavailable" | "app_unavailable" => DesktopError::NotFound,
        "platform_unavailable" => DesktopError::Unavailable,
        _ => DesktopError::Failed,
    }
}
