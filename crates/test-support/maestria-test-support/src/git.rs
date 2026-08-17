//! Git invocation helper for fixture repositories.

use crate::error::TestSupportError;
use std::path::Path;
use std::process::Command;

/// Runs `git` in `repo` with `args`, failing with a labeled error when the
/// command exits unsuccessfully.
pub fn run_git(repo: &Path, args: &[&str], operation: &str) -> Result<(), TestSupportError> {
    let status = Command::new("git").current_dir(repo).args(args).status()?;
    if !status.success() {
        return Err(TestSupportError::new(format!(
            "{operation}: git {args:?} failed in {}",
            repo.display()
        )));
    }
    Ok(())
}
