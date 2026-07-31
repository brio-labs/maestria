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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ArtifactIdEntry {
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

pub(super) fn persist_state(layout: &InstanceLayout, state: &WatchState) -> Result<()> {
    fs::create_dir_all(&layout.system_dir).with_context(|| {
        format!(
            "create watcher state directory {}",
            layout.system_dir.display()
        )
    })?;
    let path = layout.system_dir.join(WATCH_STATE_FILE);
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(state)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}
