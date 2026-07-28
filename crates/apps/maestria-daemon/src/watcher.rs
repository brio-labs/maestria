use anyhow::{Context, Result};
use maestria_core::{InstanceLayout, InstanceManifest, artifact_id_for, content_hash};
use maestria_domain::{ArtifactDetected, DomainInput, SourceRemoved};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;

const WATCH_INTERVAL: Duration = Duration::from_secs(1);
const WATCH_STATE_FILE: &str = "watcher-state.json";

/// Maximum number of concurrent scan operations. Prevents unbounded I/O
/// when the manifest contains many read roots.
const MAX_CONCURRENT_SCANS: usize = 4;

/// Durable watch state persisted between daemon restarts for crash recovery.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct WatchState {
    files: BTreeMap<String, String>,
    removed: BTreeMap<String, String>,
    artifact_ids: BTreeMap<String, ArtifactIdEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactIdEntry {
    artifact_id: u64,
    content_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingDeliveryStatus {
    /// The input was accepted by the bounded channel, but the runtime has not
    /// durably reported acceptance yet.
    Enqueued,
    /// The input could not be queued because the bounded channel was full.
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingDelivery {
    content_hash: String,
    status: PendingDeliveryStatus,
}

#[derive(Debug, Clone)]
struct Observation {
    path: PathBuf,
    bytes: Vec<u8>,
    hash: String,
}

pub(crate) fn spawn(
    layout: InstanceLayout,
    manifest: InstanceManifest,
    input_tx: mpsc::Sender<DomainInput>,
    artifact_ids: BTreeMap<String, (maestria_domain::ArtifactId, String)>,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut state = load_state(&layout);
        // Merge startup artifact_ids from event store into persisted state.
        // Startup-provided mapping (from SQLite event replay) takes precedence
        // over potentially stale persisted entries.
        for (key, (aid, hash)) in &artifact_ids {
            state.artifact_ids.insert(
                key.clone(),
                ArtifactIdEntry {
                    artifact_id: aid.value(),
                    content_hash: hash.clone(),
                },
            );
        }
        let scan_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS));
        let watcher = Watcher {
            layout,
            manifest,
            input_tx,
            artifact_ids: artifact_ids.into_iter().collect(),
            shutdown,
            state,
            pending: BTreeMap::new(),
            scan_permits,
        };
        watcher.run().await;
    })
}

struct Watcher {
    layout: InstanceLayout,
    manifest: InstanceManifest,
    input_tx: mpsc::Sender<DomainInput>,
    artifact_ids: BTreeMap<String, (maestria_domain::ArtifactId, String)>,
    shutdown: CancellationToken,
    state: WatchState,
    /// Inputs accepted by the bounded channel but not yet confirmed by a
    /// durable runtime-derived artifact identity. This is intentionally
    /// in-memory: shutdown must not turn channel enqueue into acceptance.
    pending: BTreeMap<String, PendingDelivery>,
    scan_permits: Arc<Semaphore>,
}

impl Watcher {
    async fn run(mut self) {
        let mut ticks = interval(WATCH_INTERVAL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                _ = ticks.tick() => {
                    if let Err(error) = self.scan_once().await {
                        tracing::warn!(%error, "continuous ingestion scan failed");
                    }
                }
            }
        }
        if let Err(error) = persist_state(&self.layout, &self.state) {
            tracing::warn!(%error, "failed to persist continuous ingestion state on shutdown");
        }
    }

    async fn scan_once(&mut self) -> Result<()> {
        let permits = self.scan_permits.clone();
        let _permit = permits.acquire().await.context("acquire scan permit")?;

        let observations = scan_manifest(&self.manifest)?;
        let current = self.phase_detect_additions(&observations).await?;
        self.pending.retain(|key, pending| {
            current
                .get(key)
                .is_some_and(|hash| hash == &pending.content_hash)
        });

        // Use the observed map while detecting removals so a source that is
        // waiting for runtime acceptance is not mistaken for a deletion.
        let previous_files = std::mem::replace(&mut self.state.files, current);
        let removals_result = self.phase_detect_removals(previous_files).await;

        // Only durably retain files that were already accepted before this
        // scan. Newly enqueued or backpressured observations remain in the
        // in-memory pending map and are intentionally retried after restart.
        self.state.files.retain(|key, hash| {
            !self
                .pending
                .get(key)
                .is_some_and(|pending| pending.content_hash == *hash)
        });
        self.state.artifact_ids.retain(|key, _| {
            self.state.files.contains_key(key) || self.state.removed.contains_key(key)
        });
        removals_result?;
        persist_state(&self.layout, &self.state)
    }

    async fn phase_detect_additions(
        &mut self,
        observations: &[Observation],
    ) -> Result<BTreeMap<String, String>> {
        let mut current: BTreeMap<String, String> = BTreeMap::new();

        for observation in observations {
            let key = source_key(&observation.path);
            current.insert(key.clone(), observation.hash.clone());

            let accepted_artifact = self
                .artifact_ids
                .get(&key)
                .filter(|(_, known_hash)| known_hash.as_str() == observation.hash.as_str())
                .map(|(artifact_id, _)| *artifact_id);
            let accepted = self.state.files.get(&key) == Some(&observation.hash)
                || accepted_artifact.is_some();
            if accepted {
                if let Some(artifact_id) = accepted_artifact {
                    self.state.artifact_ids.insert(
                        key.clone(),
                        ArtifactIdEntry {
                            artifact_id: artifact_id.value(),
                            content_hash: observation.hash.clone(),
                        },
                    );
                }
                self.pending.remove(&key);
                self.state.removed.remove(&key);
                continue;
            }

            let pending_status = self.pending.get(&key).and_then(|pending| {
                (pending.content_hash == observation.hash).then_some(pending.status)
            });
            if let Some(status) = pending_status {
                if status == PendingDeliveryStatus::Enqueued {
                    self.state.removed.remove(&key);
                    continue;
                }
            } else {
                // The source changed before the previous delivery was
                // accepted; retry the newest deterministic content.
                self.pending.remove(&key);
            }

            let artifact_id = artifact_id_for(&observation.path, &observation.bytes);
            let title = match observation.path.file_name().and_then(|name| name.to_str()) {
                Some(name) => name.to_string(),
                None => "unknown".to_string(),
            };

            match self
                .input_tx
                .try_send(DomainInput::ArtifactDetected(ArtifactDetected {
                    artifact_id,
                    title,
                    source_path: key.clone(),
                    source_bytes: observation.bytes.clone(),
                    content_hash: observation.hash.clone(),
                })) {
                Ok(()) => {
                    self.pending.insert(
                        key.clone(),
                        PendingDelivery {
                            content_hash: observation.hash.clone(),
                            status: PendingDeliveryStatus::Enqueued,
                        },
                    );
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::debug!("watcher input channel full — deferring artifact detection");
                    self.pending.insert(
                        key.clone(),
                        PendingDelivery {
                            content_hash: observation.hash.clone(),
                            status: PendingDeliveryStatus::Deferred,
                        },
                    );
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(anyhow::anyhow!(
                        "submit watched artifact: input channel closed"
                    ));
                }
            }

            self.state.removed.remove(&key);
        }

        Ok(current)
    }

    async fn phase_detect_removals(
        &mut self,
        previous_files: BTreeMap<String, String>,
    ) -> Result<()> {
        let hash_index: BTreeMap<String, String> = self
            .state
            .files
            .iter()
            .map(|(k, h)| (h.clone(), k.clone()))
            .collect();

        for (prev_key, prev_hash) in &previous_files {
            if self.state.files.contains_key(prev_key) {
                continue;
            }

            if let Some(new_key) = hash_index.get(prev_hash.as_str())
                && new_key != prev_key
            {
                tracing::info!(
                    from = %prev_key,
                    to = %new_key,
                    "watcher detected file rename"
                );
            }

            self.state
                .removed
                .entry(prev_key.clone())
                .or_insert_with(|| prev_hash.clone());

            match self.emit_source_removed(prev_key, prev_hash)? {
                true => {}
                false => {
                    tracing::debug!(
                        source_path = %prev_key,
                        "deferring SourceRemoved emission (channel full or missing artifact id)"
                    );
                    if self.state.artifact_ids.contains_key(prev_key) {
                        self.state.files.insert(prev_key.clone(), prev_hash.clone());
                    }
                }
            }
        }

        self.state.artifact_ids.retain(|key, _| {
            self.state.files.contains_key(key) || self.state.removed.contains_key(key)
        });

        Ok(())
    }

    fn emit_source_removed(&self, prev_key: &str, _prev_hash: &str) -> Result<bool> {
        let Some(ArtifactIdEntry {
            artifact_id: aid_val,
            content_hash: entry_hash,
        }) = self.state.artifact_ids.get(prev_key)
        else {
            return Ok(false);
        };

        match self
            .input_tx
            .try_send(DomainInput::SourceRemoved(SourceRemoved {
                artifact_id: maestria_domain::ArtifactId::new(*aid_val),
                source_path: prev_key.to_string(),
                content_hash: entry_hash.clone(),
            })) {
            Ok(()) => Ok(true),
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::debug!(
                    source_path = %prev_key,
                    "channel full, deferring SourceRemoved emission"
                );
                Ok(false)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(anyhow::anyhow!(
                "submit source removed: input channel closed"
            )),
        }
    }
}

fn source_key(path: &Path) -> String {
    match path.canonicalize() {
        Ok(path) => path.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

fn load_state(layout: &InstanceLayout) -> WatchState {
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

fn persist_state(layout: &InstanceLayout, state: &WatchState) -> Result<()> {
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

/// Scan manifest roots using `ignore::WalkBuilder` for gitignore/.ignore-aware
/// traversal. The walker respects `.gitignore`, `.ignore`, and hidden-file
/// conventions automatically.
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn is_instance_path(path: &Path, normalized_instance_root: &Path) -> bool {
    normalize_path(path).starts_with(normalized_instance_root)
}

fn is_instance_internal_path(path: &Path, normalized_instance_root: &Path) -> bool {
    let normalized_path = normalize_path(path);
    let Some(relative) = normalized_path.strip_prefix(normalized_instance_root).ok() else {
        return false;
    };
    let Some(first) = relative.components().next() else {
        return false;
    };
    matches!(
        first,
        Component::Normal(name)
            if matches!(name.to_str(), Some("system" | "indexes" | "blobs" | "manifest.txt"))
    )
}

fn scan_manifest(manifest: &InstanceManifest) -> Result<Vec<Observation>> {
    let mut observations = Vec::new();
    let instance_root = manifest.root.clone();
    let normalized_instance_root = normalize_path(&instance_root);

    for root in &manifest.read_roots {
        let root = root.clone();
        let normalized_root = normalize_path(&root);
        let exclude_instance = normalized_root != normalized_instance_root
            && normalized_instance_root.starts_with(&normalized_root);
        let normalized_instance_root = normalized_instance_root.clone();
        let walker = ignore::WalkBuilder::new(root)
            .filter_entry(move |entry| {
                if exclude_instance {
                    !is_instance_path(entry.path(), &normalized_instance_root)
                } else {
                    !is_instance_internal_path(entry.path(), &normalized_instance_root)
                }
            })
            .hidden(true)
            .ignore(true)
            .git_ignore(true)
            .git_global(false)
            .require_git(false)
            .follow_links(false)
            .build();
        for result in walker {
            let entry = result?;
            if let Some(error) = entry.error() {
                return Err(anyhow::anyhow!(
                    "traversal error at {}: {error}",
                    entry.path().display()
                ));
            }
            let path = entry.path().to_path_buf();

            // Skip symlinks and non-files.
            if !entry
                .file_type()
                .is_some_and(|ft| ft.is_file() && !ft.is_symlink())
            {
                continue;
            }

            // Enforce manifest scoping (excluded patterns + read root checks).
            if !manifest.allows_source(&path) {
                continue;
            }

            // Only supported document extensions.
            if !is_supported_file(&path) {
                continue;
            }

            let bytes =
                fs::read(&path).with_context(|| format!("read watched file {}", path.display()))?;
            observations.push(Observation {
                path,
                hash: content_hash(&bytes),
                bytes,
            });
        }
    }
    observations.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(observations)
}

fn is_supported_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown" | "txt" | "rs" | "toml" | "json" | "yaml" | "yml" | "pdf")
    )
}

#[cfg(test)]
fn test_manifest(root: PathBuf) -> InstanceManifest {
    InstanceManifest {
        schema_version: 1,
        root: root.clone(),
        read_roots: vec![root],
        excluded_patterns: vec![".env".to_string()],
        embeddings: None,
        ocr: None,
        visual: None,
    }
}

#[cfg(test)]
#[path = "watcher_tests.rs"]
mod watcher_tests;

#[cfg(test)]
#[path = "watcher_removal_tests.rs"]
mod watcher_removal_tests;
