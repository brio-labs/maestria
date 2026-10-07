//! Filesystem helpers for fixture trees.

use crate::error::TestSupportError;
use std::fs;
use std::path::Path;

/// Recursively copies `source` into `target`, preserving the directory
/// structure and file contents.
pub fn copy_tree(source: &Path, target: &Path) -> Result<(), TestSupportError> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let child = entry.path();
        let destination = target.join(entry.file_name());
        if child.is_dir() {
            copy_tree(&child, &destination)?;
        } else {
            fs::copy(&child, &destination)?;
        }
    }
    Ok(())
}
