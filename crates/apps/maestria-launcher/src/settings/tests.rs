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
        warning.contains("could not be read") && warning.contains("invalid shortcut accelerator")
    }));
    assert!(!settings.read_only);
    let update = PreferencesUpdate {
        reduce_motion: Some(true),
        theme: None,
        shortcut: None,
        shortcut_setup: None,
        confirm_reset: None,
    };
    settings
        .apply_update(&update)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    assert!(settings.values.reduce_motion);
    assert!(
        settings
            .warning
            .as_deref()
            .is_some_and(|warning| { warning.contains("session-only") })
    );
    assert_eq!(fs::read_to_string(&path)?, contents);
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn malformed_preferences_accept_session_only_changes_without_overwrite() -> io::Result<()> {
    let directory = test_directory()?;
    let path = directory.join(SETTINGS_FILE);
    let contents = "schemaVersion = 1\nshortcut = [\n";
    fs::write(&path, contents)?;

    let mut settings = SettingsManager::load(Ok(directory.clone()));
    settings
        .apply_update(&PreferencesUpdate {
            reduce_motion: Some(true),
            theme: Some("dark".to_string()),
            shortcut: None,
            shortcut_setup: None,
            confirm_reset: None,
        })
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    assert!(settings.values.reduce_motion);
    assert_eq!(settings.theme(), "dark");
    assert!(
        settings
            .warning
            .as_deref()
            .is_some_and(|warning| { warning.contains("session-only") })
    );
    assert_eq!(fs::read_to_string(&path)?, contents);
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn future_schema_preferences_are_read_only_and_preserved() -> io::Result<()> {
    let directory = test_directory()?;
    let path = directory.join(SETTINGS_FILE);
    let contents = "schemaVersion = 2\nfutureSetting = \"keep this value\"\n";
    fs::write(&path, contents)?;

    let mut settings = SettingsManager::load(Ok(directory.clone()));
    assert!(settings.read_only);
    assert!(settings.warning.is_some());
    assert!(
        settings
            .apply_update(&PreferencesUpdate {
                reduce_motion: Some(true),
                theme: Some("dark".to_string()),
                shortcut: None,
                shortcut_setup: None,
                confirm_reset: None,
            })
            .is_err()
    );
    assert!(settings.confirm_reset().is_err());
    assert!(!settings.values.reduce_motion);
    assert_eq!(settings.theme(), "system");
    assert_eq!(fs::read_to_string(&path)?, contents);
    fs::remove_dir_all(directory)?;
    Ok(())
}
