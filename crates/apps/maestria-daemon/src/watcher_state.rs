use anyhow::{Context, Result};
use maestria_core::InstanceLayout;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;

pub(super) const WATCH_STATE_FILE: &str = "watcher-state.json";
/// Durable watch state persisted between daemon restarts for crash recovery.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct WatchState {
    pub(super) files: BTreeMap<String, String>,
    pub(super) removed: BTreeMap<String, String>,
    pub(super) artifact_ids: BTreeMap<String, ArtifactIdEntry>,
    #[serde(default)]
    pub(super) pending_removals: BTreeMap<String, PendingRemovalEntry>,
    /// Lightweight per-file change signatures (mtime, size) used to skip
    /// re-reading unchanged files on every scan (issue #440). Absent in
    /// state persisted by older versions; the first scan after an upgrade
    /// re-reads everything once.
    #[serde(default)]
    pub(super) signatures: BTreeMap<String, FileSignature>,
    /// Persisted indexing diagnostics survive daemon restarts.
    #[serde(default)]
    pub(super) last_scan_unix_ms: Option<u64>,
    #[serde(default)]
    pub(super) last_error: Option<String>,
    #[serde(default)]
    pub(super) pending_files: usize,
    #[serde(default)]
    pub(super) scanning: bool,
}

/// Change signature of an observed file, including metadata that changes on
/// writes even when a caller restores the mtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FileSignature {
    pub(super) mtime: i64,
    #[serde(default)]
    pub(super) change_time: i64,
    pub(super) size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ArtifactIdEntry {
    pub(super) artifact_id: u64,
    pub(super) content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PendingRemovalEntry {
    pub(super) source_path: String,
    pub(super) artifact_id: u64,
    pub(super) content_hash: String,
}

pub(super) fn load_state(layout: &InstanceLayout) -> WatchState {
    let path = layout.system_dir.join(WATCH_STATE_FILE);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return WatchState::default();
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "failed to read persisted watcher state; starting with empty state"
            );
            return WatchState::default();
        }
    };
    match serde_json::from_str(&contents) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "failed to decode persisted watcher state; starting with empty state"
            );
            WatchState::default()
        }
    }
}

pub(super) fn unix_time_millis() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => {
            // Cap before narrowing: a timestamp beyond u64 milliseconds must saturate.
            duration.as_millis().min(u128::from(u64::MAX)) as u64
        }
        Err(_) => 0,
    }
}

pub(super) fn persist_state(layout: &InstanceLayout, state: &WatchState) -> Result<()> {
    fs::create_dir_all(&layout.system_dir).with_context(|| {
        format!(
            "create watcher state directory {}",
            layout.system_dir.display()
        )
    })?;
    let path = layout.system_dir.join(WATCH_STATE_FILE);
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec(state)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WatcherStatus {
    pub(crate) indexed_sources: Vec<(String, String)>,
    pub(crate) current_artifact_ids: BTreeMap<String, u64>,
    pub(crate) pending_files: usize,
    pub(crate) scanning: bool,
    pub(crate) last_scan_unix_ms: Option<u64>,
    pub(crate) last_error: Option<String>,
}

pub(crate) fn status(layout: &InstanceLayout) -> WatcherStatus {
    let state = load_state(layout);
    let current_artifact_ids = state
        .artifact_ids
        .iter()
        .filter_map(|(path, entry)| {
            state
                .files
                .get(path)
                .filter(|content_hash| *content_hash == &entry.content_hash)
                .map(|_| (path.clone(), entry.artifact_id))
        })
        .collect();
    WatcherStatus {
        indexed_sources: state.files.into_iter().collect(),
        current_artifact_ids,
        pending_files: state.pending_files,
        scanning: state.scanning,
        last_scan_unix_ms: state.last_scan_unix_ms,
        last_error: state.last_error,
    }
}
