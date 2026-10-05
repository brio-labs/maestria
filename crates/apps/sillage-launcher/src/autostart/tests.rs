use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "sillage-autostart-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(Self(path))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn entry_path(directory: &TestDirectory) -> PathBuf {
    directory
        .0
        .join("shared-config")
        .join("fresh")
        .join("nested")
        .join("autostart")
        .join(FILE_NAME)
}

#[test]
fn fresh_config_ancestors_are_private_and_opt_in_is_reversible()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::new()?;
    let shared_config = directory.0.join("shared-config");
    fs::create_dir(&shared_config)?;
    fs::set_permissions(&shared_config, fs::Permissions::from_mode(0o755))?;
    let path = entry_path(&directory);
    let shared_mode = fs::symlink_metadata(&shared_config)?.permissions().mode() & 0o777;

    assert!(!enabled_at(&path)?);
    set_enabled_at(&path, true)?;
    let contents = fs::read(&path)?;
    assert_eq!(contents.as_slice(), CONTENT.as_bytes());
    assert!(enabled_at(&path)?);
    assert_eq!(
        fs::symlink_metadata(&shared_config)?.permissions().mode() & 0o777,
        shared_mode
    );
    for ancestor in ["fresh", "fresh/nested", "fresh/nested/autostart"] {
        let metadata = fs::symlink_metadata(shared_config.join(ancestor))?;
        assert!(metadata.is_dir());
        assert_eq!(metadata.permissions().mode() & 0o077, 0);
    }

    set_enabled_at(&path, false)?;
    assert!(!enabled_at(&path)?);
    assert!(!path.exists());
    Ok(())
}

#[test]
fn foreign_and_symlink_entries_are_never_replaced_or_removed()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new()?;
    let path = entry_path(&directory);
    let parent = path
        .parent()
        .ok_or_else(|| LauncherError::settings_failed("The autostart directory is unavailable"))?;
    create_directories(parent)?;
    let target = directory.0.join("foreign-target");
    fs::write(&target, b"user-owned entry")?;
    symlink(&target, &path)?;

    assert!(enabled_at(&path).is_err());
    assert!(set_enabled_at(&path, true).is_err());
    assert!(set_enabled_at(&path, false).is_err());
    assert!(fs::symlink_metadata(&path)?.file_type().is_symlink());
    assert_eq!(fs::read(&target)?, b"user-owned entry");
    fs::remove_file(&path)?;
    fs::write(&path, b"user-managed entry")?;
    assert!(enabled_at(&path).is_err());
    assert!(set_enabled_at(&path, true).is_err());
    assert!(set_enabled_at(&path, false).is_err());
    assert_eq!(fs::read(&path)?, b"user-managed entry");
    Ok(())
}

#[test]
fn replacement_inode_is_preserved_during_owned_entry_removal()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::new()?;
    let path = entry_path(&directory);
    set_enabled_at(&path, true)?;
    let owned = owned_entry(&path)?
        .ok_or_else(|| LauncherError::settings_failed("Launcher entry is unavailable"))?;
    let original_inode = File::open(&path)?;
    fs::remove_file(&path)?;
    fs::write(&path, b"replacement user entry")?;

    assert!(!remove_owned_entry(&path, &owned)?);
    assert_eq!(fs::read(&path)?, b"replacement user entry");
    drop(original_inode);
    Ok(())
}
