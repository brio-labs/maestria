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
            std::env::temp_dir().join(format!("maestria-daemon-test-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path)?;
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
