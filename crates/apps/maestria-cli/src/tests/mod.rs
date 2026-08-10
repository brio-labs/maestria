//! Shared fixture helpers for CLI unit tests.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

mod observability_tests;
mod recovery_tests;
mod task_workspace_tests;

static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

pub(super) struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    pub(super) fn create() -> Result<Self, Box<dyn std::error::Error>> {
        let base = std::env::temp_dir();
        for _ in 0..1000 {
            let id = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!(
                "maestria-cli-index-test-{}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(format!("create unique test directory under {}", base.display()).into())
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
