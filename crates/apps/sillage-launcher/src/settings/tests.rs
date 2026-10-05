use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

fn test_directory() -> io::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "sillage-settings-{}-{}",
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

#[test]
fn legacy_external_search_configuration_loads_without_managed_consent() -> io::Result<()> {
    let directory = test_directory()?;
    let path = directory.join(SETTINGS_FILE);
    let realm = "a".repeat(64);
    let contents = format!(
        r#"
schemaVersion = 1
shortcut = "Control+Space"
shortcutSetup = "unconfigured"
reduceMotion = false
theme = "system"

[search]
socketPath = "/tmp/sillage-search.sock"
consumerRealm = "{realm}"
credentialFile = "/tmp/sillage-search.token"
"#
    );
    fs::write(&path, contents)?;

    let settings = SettingsManager::load(Ok(directory.clone()));

    assert!(settings.warning.is_none());
    assert_eq!(settings.managed_search(), None);
    let service = settings.search_service().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "legacy search service missing")
    })?;
    assert_eq!(
        service.socket_path,
        PathBuf::from("/tmp/sillage-search.sock")
    );
    assert_eq!(service.consumer_realm, realm);
    assert_eq!(
        service.credential_file,
        PathBuf::from("/tmp/sillage-search.token")
    );
    assert_eq!(service.program, None);
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn explicit_managed_search_consent_is_persisted() -> io::Result<()> {
    let directory = test_directory()?;
    let profile_root = directory.join("data/sillage");
    let service = SearchServiceConfig {
        socket_path: profile_root.join("system/daemon.sock"),
        consumer_realm: "b".repeat(64),
        credential_file: profile_root.join("system/launcher-search-0123456789abcdef.credential"),
        program: Some(PathBuf::from("/usr/bin/sillage-search")),
    };
    let managed = ManagedSearchConfig {
        enabled: true,
        root: PathBuf::from("/tmp/approved-documents"),
        profile_root,
        profile_identity: "a".repeat(64),
        grant_token_digest: "c".repeat(64),
        grant_expires_at_unix_seconds: 1_800_000_000,
    };
    let mut settings = SettingsManager::load(Ok(directory.clone()));
    let revision = settings.search_revision();
    settings
        .set_search_configuration_if_revision(
            revision,
            Some(service.clone()),
            Some(managed.clone()),
        )
        .map_err(io::Error::other)?;

    let reloaded = SettingsManager::load(Ok(directory.clone()));

    assert_eq!(reloaded.search_service(), Some(service));
    assert_eq!(reloaded.managed_search(), Some(managed));
    assert!(
        reloaded
            .managed_search()
            .is_some_and(|managed| managed.enabled)
    );
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn managed_search_settings_roll_back_when_persistence_fails() -> io::Result<()> {
    let directory = test_directory()?;
    let mut settings = SettingsManager::load(Ok(directory.clone()));
    let profile_root = directory.join("managed-profile");
    let service = SearchServiceConfig {
        socket_path: profile_root.join("system/daemon.sock"),
        consumer_realm: "b".repeat(64),
        credential_file: profile_root.join("system/launcher-search-0123456789abcdef.credential"),
        program: Some(PathBuf::from("/usr/bin/sillage-search")),
    };
    let managed = ManagedSearchConfig {
        enabled: true,
        root: PathBuf::from("/tmp/approved-documents"),
        profile_root,
        profile_identity: "a".repeat(64),
        grant_token_digest: "c".repeat(64),
        grant_expires_at_unix_seconds: 1_800_000_000,
    };
    let revision = settings.search_revision();
    let blocked_temporary = directory.join(format!(".{SETTINGS_FILE}.tmp-{}", std::process::id()));
    fs::create_dir(&blocked_temporary)?;

    assert!(
        settings
            .set_search_configuration_if_revision(revision, Some(service), Some(managed))
            .is_err()
    );
    assert_eq!(settings.search_service(), None);
    assert_eq!(settings.managed_search(), None);
    assert_eq!(settings.search_revision(), revision);
    assert!(settings.warning.is_some());

    fs::remove_dir_all(directory)?;
    Ok(())
}
