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

/// Load the profile at `path`; `Ok(None)` only when the file does not
/// exist. Any other read failure propagates so it is not mistaken for an
/// absent profile; malformed content propagates as an error.
pub fn load_profile(path: &Path) -> Result<Option<IndexSelectionProfile>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
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
