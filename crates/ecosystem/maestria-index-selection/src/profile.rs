//! Persisted selection profiles: a root, its whitelisted directories, and
//! per-directory policy overrides.

use crate::policy::IndexPolicy;
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// An approved selection: the directories whitelisted under `root`, plus
/// per-directory policy overrides.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexSelectionProfile {
    pub root: PathBuf,
    /// Whitelisted directories.
    pub includes: Vec<PathBuf>,
    /// Per-directory policy overrides.
    pub policies: BTreeMap<PathBuf, IndexPolicy>,
}

/// Load the profile at `path`; `Ok(None)` when the file does not exist.
/// Malformed content propagates as an error.
pub fn load_profile(path: &Path) -> Result<Option<IndexSelectionProfile>> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    let profile = serde_json::from_str(&contents)?;
    Ok(Some(profile))
}

/// Write the profile to `path` as pretty JSON.
pub fn save_profile(path: &Path, profile: &IndexSelectionProfile) -> Result<()> {
    let contents = serde_json::to_string_pretty(profile)?;
    std::fs::write(path, contents)?;
    Ok(())
}
