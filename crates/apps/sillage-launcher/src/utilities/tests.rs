use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Utilities, UtilityKind};

static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

pub(super) fn test_directory() -> io::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "sillage-utilities-test-{}-{}",
        std::process::id(),
        NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path)?;
    Ok(path)
}

pub(super) fn entry_rows(
    utilities: &mut Utilities,
    kind: UtilityKind,
) -> Vec<(String, String, String)> {
    let mut rows = Vec::new();
    utilities.visit_entries(kind, |entry| {
        rows.push((entry.id.clone(), entry.title.clone(), entry.content.clone()));
    });
    rows
}

#[path = "tests/clipboard.rs"]
mod clipboard;
#[path = "tests/expansion.rs"]
mod expansion;
#[path = "tests/persistence.rs"]
mod persistence;
#[path = "tests/store.rs"]
mod store;
