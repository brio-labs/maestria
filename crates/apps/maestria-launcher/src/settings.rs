use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{
    AcceleratorPresentation, PreferencesDto, PreferencesUpdate, PrimaryModifier, ShortcutSetup,
    ShortcutStatus,
};

const SCHEMA_VERSION: u32 = 1;
const SETTINGS_FILE: &str = "launcher.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSettings {
    schema_version: u32,
    shortcut: String,
    shortcut_setup: ShortcutSetup,
    reduce_motion: bool,
}

#[derive(Debug, Clone)]
pub struct SettingsManager {
    path: Option<PathBuf>,
    values: PersistedSettings,
    warning: Option<String>,
    read_only: bool,
    preserve_existing: bool,
    reset_confirmed: bool,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            shortcut: "Control+Space".to_string(),
            shortcut_setup: ShortcutSetup::Unconfigured,
            reduce_motion: false,
        }
    }
}

impl SettingsManager {
    pub fn load(config_dir: Result<PathBuf, String>) -> Self {
        let path = config_dir
            .ok()
            .map(|directory| directory.join(SETTINGS_FILE));
        let Some(path) = path.clone() else {
            return Self {
                path: None,
                values: PersistedSettings::default(),
                warning: Some("Preferences are session-only because the configuration directory is unavailable".to_string()),
                read_only: false,
                preserve_existing: false,
                reset_confirmed: false,
            };
        };

        match fs::read_to_string(&path) {
            Ok(contents) => match parse_settings(&contents) {
                Ok(values) => Self {
                    path: Some(path),
                    values,
                    warning: None,
                    read_only: false,
                    preserve_existing: false,
                    reset_confirmed: false,
                },
                Err(ParseSettingsError::UnknownVersion(version)) => Self {
                    path: Some(path),
                    values: PersistedSettings::default(),
                    warning: Some(format!(
                        "Preferences use unsupported schema version {version}; this file is read-only and was preserved"
                    )),
                    read_only: true,
                    preserve_existing: true,
                    reset_confirmed: false,
                },
                Err(ParseSettingsError::Malformed(message)) => Self {
                    path: Some(path),
                    values: PersistedSettings::default(),
                    warning: Some(format!(
                        "Preferences could not be read and were preserved: {message}"
                    )),
                    read_only: false,
                    preserve_existing: true,
                    reset_confirmed: false,
                },
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self {
                path: Some(path),
                values: PersistedSettings::default(),
                warning: None,
                read_only: false,
                preserve_existing: false,
                reset_confirmed: false,
            },
            Err(error) => Self {
                path: Some(path),
                values: PersistedSettings::default(),
                warning: Some(format!(
                    "Preferences could not be read and were preserved: {error}"
                )),
                read_only: false,
                preserve_existing: true,
                reset_confirmed: false,
            },
        }
    }

    pub fn dto(&self, shortcut_status: ShortcutStatus) -> PreferencesDto {
        PreferencesDto {
            schema_version: self.values.schema_version,
            shortcut: self.values.shortcut.clone(),
            shortcut_setup: self.values.shortcut_setup.clone(),
            reduce_motion: self.values.reduce_motion,
            warning: self.warning.clone(),
            read_only: self.read_only,
            platform: platform_name().to_string(),
            accelerators: if cfg!(target_os = "macos") {
                AcceleratorPresentation {
                    primary_modifier: PrimaryModifier::Meta,
                    primary_label: "Cmd",
                }
            } else {
                AcceleratorPresentation {
                    primary_modifier: PrimaryModifier::Control,
                    primary_label: "Ctrl",
                }
            },
            shortcut_status,
        }
    }

    pub fn validate_update(&self, request: &PreferencesUpdate) -> Result<(), String> {
        if self.read_only {
            return Err("Preferences from an unknown schema version are read-only".to_string());
        }
        if let Some(shortcut) = request.shortcut.as_deref()
            && (shortcut.is_empty() || shortcut.contains('\0'))
        {
            return Err("Shortcut cannot be empty or contain NUL".to_string());
        }
        Ok(())
    }

    pub fn apply_update(&mut self, request: &PreferencesUpdate) -> Result<(), String> {
        self.validate_update(request)?;
        if let Some(shortcut) = request.shortcut.as_ref() {
            self.values.shortcut.clone_from(shortcut);
        }
        if let Some(setup) = request.shortcut_setup.as_ref() {
            self.values.shortcut_setup = setup.clone();
        }
        if let Some(reduce_motion) = request.reduce_motion {
            self.values.reduce_motion = reduce_motion;
        }
        // Persistence warnings remain visible while session values take effect.
        let _ = self.persist();
        Ok(())
    }

    pub fn confirm_reset(&mut self) -> Result<(), String> {
        if self.read_only {
            return Err("Preferences from an unknown schema version are read-only".to_string());
        }
        self.reset_confirmed = true;
        Ok(())
    }

    pub(crate) fn ensure_reset_confirmed(&self) -> Result<(), String> {
        if self.reset_confirmed {
            Ok(())
        } else {
            Err("Reset requires explicit confirmation".to_string())
        }
    }

    pub fn reset(&mut self) -> Result<(), String> {
        self.ensure_reset_confirmed()?;
        self.values = PersistedSettings::default();
        self.warning = None;
        self.read_only = false;
        self.preserve_existing = false;
        self.reset_confirmed = false;
        let _ = self.persist();
        Ok(())
    }

    pub fn shortcut_setup(&self) -> ShortcutSetup {
        self.values.shortcut_setup.clone()
    }

    pub fn shortcut(&self) -> &str {
        &self.values.shortcut
    }

    fn persist(&mut self) -> Result<(), String> {
        if self.preserve_existing {
            self.warning = Some("The unreadable preferences file was preserved. Changes are session-only until Reset is confirmed".to_string());
            return Err(
                "The existing preferences file was preserved; confirm Reset to enable persistence"
                    .to_string(),
            );
        }
        let Some(path) = self.path.as_ref() else {
            self.warning = Some(
                "Preferences are session-only because the configuration directory is unavailable"
                    .to_string(),
            );
            return Ok(());
        };
        atomic_replace(path, &self.values).map_err(|error| {
            self.warning = Some(format!("Preferences could not be persisted: {error}"));
            error.to_string()
        })?;
        self.warning = None;
        Ok(())
    }
}

fn parse_settings(contents: &str) -> Result<PersistedSettings, ParseSettingsError> {
    let value = toml::from_str::<toml::Value>(contents)
        .map_err(|error| ParseSettingsError::Malformed(error.to_string()))?;
    let version = value
        .get("schemaVersion")
        .and_then(toml::Value::as_integer)
        .ok_or_else(|| ParseSettingsError::Malformed("missing schemaVersion".to_string()))?;
    if version != i64::from(SCHEMA_VERSION) {
        return Err(ParseSettingsError::UnknownVersion(version));
    }
    let settings: PersistedSettings = toml::from_str(contents)
        .map_err(|error| ParseSettingsError::Malformed(error.to_string()))?;
    validate_shortcut(&settings.shortcut).map_err(ParseSettingsError::Malformed)?;
    Ok(settings)
}

fn validate_shortcut(shortcut: &str) -> Result<(), String> {
    if shortcut.trim().is_empty() || shortcut.contains('\0') {
        return Err("Shortcut cannot be empty or contain NUL".to_string());
    }
    #[cfg(target_os = "linux")]
    {
        shortcut
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .map(|_| ())
            .map_err(|error| format!("invalid shortcut accelerator: {error}"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn test_directory() -> io::Result<PathBuf> {
        let path = std::env::temp_dir().join(format!(
            "maestria-settings-{}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(path)
    }

    #[test]
    fn valid_version_one_preferences_are_loaded() -> io::Result<()> {
        let directory = test_directory()?;
        let contents = r#"
schemaVersion = 1
shortcut = "Control+Space"
shortcutSetup = "requested"
reduceMotion = true
"#;
        fs::write(directory.join(SETTINGS_FILE), contents)?;

        let settings = SettingsManager::load(Ok(directory.clone()));

        assert_eq!(settings.shortcut(), "Control+Space");
        assert_eq!(settings.shortcut_setup(), ShortcutSetup::Requested);
        assert!(settings.values.reduce_motion);
        assert!(settings.warning.is_none());
        assert!(!settings.read_only);
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn invalid_persisted_shortcut_is_rejected_and_preserved() -> io::Result<()> {
        let directory = test_directory()?;
        let path = directory.join(SETTINGS_FILE);
        let contents = r#"
schemaVersion = 1
shortcut = "Control+NotARealKey"
shortcutSetup = "requested"
reduceMotion = false
"#;
        fs::write(&path, contents)?;

        let mut settings = SettingsManager::load(Ok(directory.clone()));

        assert_eq!(settings.shortcut(), "Control+Space");
        assert_eq!(settings.shortcut_setup(), ShortcutSetup::Unconfigured);
        assert!(settings.warning.as_deref().is_some_and(|warning| {
            warning.contains("could not be read")
                && warning.contains("invalid shortcut accelerator")
        }));
        assert!(!settings.read_only);
        settings
            .apply_update(&PreferencesUpdate {
                reduce_motion: Some(true),
                shortcut: None,
                shortcut_setup: None,
                confirm_reset: None,
            })
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        assert_eq!(fs::read_to_string(&path)?, contents);
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}

fn atomic_replace(path: &Path, settings: &PersistedSettings) -> io::Result<()> {
    let directory = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "settings path has no parent directory",
        )
    })?;
    fs::create_dir_all(directory)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "settings path has no valid filename",
            )
        })?;
    let temporary = directory.join(format!(".{file_name}.tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .truncate(true)
        .open(&temporary)?;
    let contents = toml::to_string_pretty(settings)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    if let Err(error) = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
    {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    drop(file);
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

#[derive(Debug)]
enum ParseSettingsError {
    UnknownVersion(i64),
    Malformed(String),
}
