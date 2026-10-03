use std::str::FromStr;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use tokio::sync::mpsc as async_mpsc;

use super::{
    DEFAULT_SHORTCUT, X11Command, available_status, lock, unavailable_status, unconfigured_status,
};
use crate::errors::LauncherError;
use crate::model::{ShortcutConfigureAction, ShortcutControl, ShortcutStatus};

pub(super) async fn x11_worker(
    mut receiver: async_mpsc::Receiver<X11Command>,
    status: Arc<Mutex<ShortcutStatus>>,
    activation: mpsc::SyncSender<()>,
) {
    let manager = match global_hotkey::GlobalHotKeyManager::new() {
        Ok(manager) => manager,
        Err(error) => {
            *lock(&status) =
                unavailable_status(&format!("X11 global shortcuts are unavailable: {error}"));
            return;
        }
    };
    let mut current: Option<HotKey> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    loop {
        tokio::select! {
            command = receiver.recv() => match command {
                Some(X11Command::Configure { hotkey, response }) => {
                    let result = if current == Some(hotkey) {
                        Ok(available_status(
                            ShortcutControl::Application,
                            &hotkey.to_string(),
                            None,
                        ))
                    } else {
                        match manager.register(hotkey) {
                            Err(error) => Err(LauncherError::new(
                                "shortcut_unavailable",
                                format!("The shortcut could not be registered: {error}"),
                                true,
                            )),
                            Ok(()) => {
                                if let Some(previous) = current
                                    && let Err(error) = manager.unregister(previous)
                                {
                                    let _ = manager.unregister(hotkey);
                                    Err(LauncherError::new(
                                        "shortcut_unavailable",
                                        format!(
                                            "The previous shortcut could not be released: {error}"
                                        ),
                                        true,
                                    ))
                                } else {
                                    current = Some(hotkey);
                                    Ok(available_status(
                                        ShortcutControl::Application,
                                        &hotkey.to_string(),
                                        None,
                                    ))
                                }
                            }
                        }
                    };
                    if let Ok(updated) = result.as_ref() {
                        *lock(&status) = updated.clone();
                    }
                    let _ = response.send(result);
                }
                Some(X11Command::Clear { response }) => {
                    let result = match current {
                        Some(previous) => match manager.unregister(previous) {
                            Ok(()) => {
                                current = None;
                                Ok(())
                            }
                            Err(error) => Err(LauncherError::new(
                                "shortcut_unavailable",
                                format!("The shortcut could not be released: {error}"),
                                true,
                            )),
                        },
                        None => Ok(()),
                    };
                    let result = result.map(|()| {
                        unconfigured_status(
                            ShortcutControl::Application,
                            DEFAULT_SHORTCUT,
                            None,
                            ShortcutConfigureAction::Setup,
                        )
                    });
                    if let Ok(updated) = result.as_ref() {
                        *lock(&status) = updated.clone();
                    }
                    let _ = response.send(result);
                }
                Some(X11Command::Shutdown) | None => {
                    if let Some(previous) = current.take() {
                        let _ = manager.unregister(previous);
                    }
                    return;
                }
            },
            _ = tick.tick() => {
                drain_pressed_events(current, &activation);
            }
        }
    }
}

fn drain_pressed_events(current: Option<HotKey>, activation: &mpsc::SyncSender<()>) {
    loop {
        match GlobalHotKeyEvent::receiver().try_recv() {
            Ok(event)
                if current.is_some_and(|hotkey| event.id() == hotkey.id())
                    && event.state() == HotKeyState::Pressed =>
            {
                let _ = activation.try_send(());
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

pub(in crate::shortcuts) fn parse_hotkey(value: &str) -> Result<HotKey, String> {
    let mut parts = value.split('+').map(str::trim).peekable();
    let mut normalized = Vec::new();
    let mut key = None;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            key = Some(part);
            break;
        }
        let modifier = match part.to_ascii_lowercase().as_str() {
            "control" | "ctrl" => "ctrl",
            "alt" | "option" => "alt",
            "shift" => "shift",
            "super" | "meta" | "cmd" | "command" => "super",
            _ => return Err(format!("unknown modifier {part}")),
        };
        normalized.push(modifier.to_string());
    }
    let key = key.ok_or_else(|| "shortcut has no trigger key".to_string())?;
    normalized.push(normalize_key(key)?);
    HotKey::from_str(&normalized.join("+"))
        .map_err(|error| format!("invalid shortcut accelerator: {error}"))
}

fn normalize_key(key: &str) -> Result<String, String> {
    if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() {
        return Ok(format!("Key{}", key.to_ascii_uppercase()));
    }
    if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() {
        return Ok(format!("Digit{key}"));
    }
    let normalized = match key.to_ascii_lowercase().as_str() {
        "space" => "Space".to_string(),
        "enter" | "return" => "Enter".to_string(),
        "esc" | "escape" => "Escape".to_string(),
        "backspace" => "Backspace".to_string(),
        "tab" => "Tab".to_string(),
        "delete" => "Delete".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "pageup" | "page up" => "PageUp".to_string(),
        "pagedown" | "page down" => "PageDown".to_string(),
        "left" => "ArrowLeft".to_string(),
        "right" => "ArrowRight".to_string(),
        "up" => "ArrowUp".to_string(),
        "down" => "ArrowDown".to_string(),
        _ if key.starts_with('F') && key[1..].chars().all(|ch| ch.is_ascii_digit()) => {
            key.to_string()
        }
        _ if key.starts_with("Key") || key.starts_with("Digit") || key.starts_with("Numpad") => {
            key.to_string()
        }
        _ => return Err(format!("unknown key {key}")),
    };
    Ok(normalized)
}

pub(super) fn validate_accelerator(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.as_bytes().contains(&0) {
        return Err("shortcut cannot be empty or contain NUL".to_string());
    }
    parse_hotkey(value).map(|_| ())
}
