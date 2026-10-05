use std::error::Error;

use super::super::persistence::UTILITIES_FILE;
use super::super::{Utilities, UtilityKind};
use super::test_directory;

#[test]
fn failed_write_preserves_live_entries_and_external_file_bytes() -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let mut utilities = Utilities::load(Ok(directory.clone()))?;
    utilities.upsert(UtilityKind::Snippet, "stable", "Before", "before")?;
    let path = directory.join(UTILITIES_FILE);
    let foreign = b"foreign bytes that must not be replaced";
    std::fs::write(&path, foreign)?;

    assert!(
        utilities
            .upsert(UtilityKind::Snippet, "stable", "After", "after")
            .is_err()
    );
    assert_eq!(
        utilities.entry(UtilityKind::Snippet, "stable")?.title,
        "Before"
    );
    assert_eq!(std::fs::read(&path)?, foreign);

    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn invalid_or_unexpected_files_are_preserved() -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let path = directory.join(UTILITIES_FILE);
    let future_schema = b"schemaVersion = 99\n[foreign]\nvalue = 'keep me'\n";
    std::fs::write(&path, future_schema)?;
    assert!(Utilities::load(Ok(directory.clone())).is_err());
    assert_eq!(std::fs::read(&path)?, future_schema);

    std::fs::remove_file(&path)?;
    let mut utilities = Utilities::load(Ok(directory.clone()))?;
    let foreign = b"not a utilities document";
    std::fs::write(&path, foreign)?;
    assert!(
        utilities
            .upsert(UtilityKind::Snippet, "new", "New", "body")
            .is_err()
    );
    assert_eq!(std::fs::read(&path)?, foreign);

    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn symlink_store_is_not_followed_or_replaced() -> Result<(), Box<dyn Error>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let directory = test_directory()?;
        let path = directory.join(UTILITIES_FILE);
        let target = directory.join("outside.toml");
        std::fs::write(&target, b"outside data")?;
        symlink(&target, &path)?;

        assert!(Utilities::load(Ok(directory.clone())).is_err());
        assert_eq!(std::fs::read(&target)?, b"outside data");
        assert!(std::fs::symlink_metadata(&path)?.file_type().is_symlink());

        std::fs::remove_dir_all(directory)?;
    }
    Ok(())
}
