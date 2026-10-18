//! Shared daemon test fixtures (Rule 26: fixtures are shared through
//! explicit helpers, never copied between test modules).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// A temporary directory that is removed on drop. Each instance gets a
/// unique path via a process-wide atomic counter so test modules running in
/// the same process never collide.
pub(crate) struct TempDir(PathBuf);
impl TempDir {
    pub(crate) fn create() -> std::io::Result<Self> {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("sillage-daemon-test-{}-{id}", std::process::id()));
        Self::create_at(path)
    }

    fn create_at(path: PathBuf) -> std::io::Result<Self> {
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::TempDir;
    use std::{fs, io};

    #[test]
    fn fixture_creation_exclusively_owns_a_fresh_directory() -> io::Result<()> {
        let parent = TempDir::create()?;
        let path = parent.path().join("fresh-fixture");
        let fixture = TempDir::create_at(path.clone())?;
        assert!(fs::read_dir(fixture.path())?.next().transpose()?.is_none());
        drop(fixture);
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn fixture_collision_preserves_the_existing_directory_and_contents() -> io::Result<()> {
        let parent = TempDir::create()?;
        let path = parent.path().join("existing-fixture");
        fs::create_dir(&path)?;
        let sentinel = path.join("owner-sentinel");
        fs::write(&sentinel, b"existing owner's contents")?;

        let error = match TempDir::create_at(path.clone()) {
            Err(error) => error,
            Ok(_) => return Err(io::Error::other("fixture adopted a preexisting directory")),
        };
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(path.is_dir());
        assert_eq!(fs::read(&sentinel)?, b"existing owner's contents");
        Ok(())
    }
}
