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

    #[expect(
        clippy::too_many_lines,
        reason = "one scan owns the durable watcher transition transaction"
    )]
    async fn scan_once(&mut self) -> Result<()> {
        // Acquire a scan permit for bounded concurrency.
        let _permit = self
            .scan_permits
            .acquire()
            .await
            .context("acquire scan permit")?;

        let observations = scan_manifest(&self.manifest)?;
        let mut current: BTreeMap<String, String> = BTreeMap::new();

        // ── Phase 1: process current files ──────────────────────────────────
        for observation in &observations {
            let key = source_key(&observation.path);
            current.insert(key.clone(), observation.hash.clone());

            if self.state.files.get(&key) == Some(&observation.hash)
                || self
                    .artifact_ids
                    .get(&key)
                    .is_some_and(|(_, known_hash)| known_hash == &observation.hash)
            {
                continue;
            }

            let artifact_id = artifact_id_for(&observation.path, &observation.bytes);
            let title = match observation.path.file_name().and_then(|name| name.to_str()) {
                Some(name) => name.to_string(),
                None => "unknown".to_string(),
            };
            let observed_hash = observation.hash.clone();

            // Non-blocking capacity check for backpressure.
            if self.input_tx.max_capacity() > 0 && self.input_tx.capacity() == 0 {
                tracing::debug!("watcher input channel full — deferring artifact detection");
                current.remove(&key);
                continue;
            }

            self.input_tx
                .send(DomainInput::ArtifactDetected(ArtifactDetected {
                    artifact_id,
                    title,
                    source_path: key.clone(),
                    source_bytes: observation.bytes.clone(),
                    content_hash: observation.hash.clone(),
                }))
                .await
                .context("submit watched artifact")?;

            self.artifact_ids
                .insert(key.clone(), (artifact_id, observed_hash));
            self.state.artifact_ids.insert(
                key.clone(),
                ArtifactIdEntry {
                    artifact_id: artifact_id.value(),
                    content_hash: observation.hash.clone(),
                },
            );
        }

        // ── Phase 2: detect removals and renames ────────────────────────────
        let previous_files = std::mem::replace(&mut self.state.files, current);

        let hash_index: BTreeMap<&str, &str> = self
            .state
            .files
            .iter()
            .map(|(k, h)| (h.as_str(), k.as_str()))
            .collect();

        for (prev_key, prev_hash) in &previous_files {
            if self.state.files.contains_key(prev_key) {
                continue;
            }

            // Rename detection: same hash at a different path.
            if let Some(&new_key) = hash_index.get(prev_hash.as_str())
                && new_key != prev_key
            {
                tracing::info!(
                    from = %prev_key,
                    to = %new_key,
                    "watcher detected file rename"
                );
            }

            // Record tombstone and emit SourceRemoved.
            self.state
                .removed
                .entry(prev_key.clone())
                .or_insert_with(|| prev_hash.clone());

            if let Some(ArtifactIdEntry {
                artifact_id: aid_val,
                content_hash: entry_hash,
            }) = self.state.artifact_ids.get(prev_key)
                && self
                    .input_tx
                    .try_send(DomainInput::SourceRemoved(SourceRemoved {
                        artifact_id: maestria_domain::ArtifactId::new(*aid_val),
                        source_path: prev_key.clone(),
                        content_hash: entry_hash.clone(),
                    }))
                    .is_err()
            {
                tracing::debug!(
                    source_path = %prev_key,
                    "channel full, deferring SourceRemoved emission"
                );
            }
        }

        // Clean up removed entries that are no longer absent.
        self.state
            .removed
            .retain(|key, _| !self.state.files.contains_key(key));

        // Prune artifact_ids for paths that no longer exist and are not in tombstone.
        self.state.artifact_ids.retain(|key, _| {
            self.state.files.contains_key(key) || self.state.removed.contains_key(key)
        });

        persist_state(&self.layout, &self.state)
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
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return WatchState::default(),
    };
    if let Ok(state) = serde_json::from_str(&contents) {
        return state;
    }
    WatchState::default()
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
mod tests {
    use super::*;
    use std::{env, process};
    use tokio::sync::mpsc;

    fn test_manifest(root: PathBuf) -> InstanceManifest {
        InstanceManifest {
            schema_version: 1,
            root: root.clone(),
            read_roots: vec![root],
            excluded_patterns: vec![".env".to_string()],
            embeddings: None,
            ocr: None,
        }
    }

    #[test]
    fn scan_skips_instance_state_when_root_contains_instance()
    -> Result<(), Box<dyn std::error::Error>> {
        let root =
            env::temp_dir().join(format!("maestria-watcher-instance-root-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        let instance = root.join("instance");
        fs::create_dir_all(instance.join("system"))?;
        fs::write(root.join("research.md"), "research")?;
        fs::write(instance.join("system").join(WATCH_STATE_FILE), "{}")?;

        let manifest = InstanceManifest {
            schema_version: 1,
            root: instance,
            read_roots: vec![root.clone()],
            excluded_patterns: Vec::new(),
            embeddings: None,
            ocr: None,
        };
        let observations = scan_manifest(&manifest)?;

        assert_eq!(observations.len(), 1);
        assert!(observations[0].path.ends_with("research.md"));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn scan_preserves_relative_manifest_scope() -> Result<(), Box<dyn std::error::Error>> {
        let root = PathBuf::from(format!(".maestria-watcher-relative-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        fs::write(root.join("note.md"), "relative note")?;

        let manifest = test_manifest(root.clone());
        let observations = scan_manifest(&manifest)?;

        assert_eq!(observations.len(), 1);
        assert!(observations[0].path.ends_with("note.md"));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn scan_allows_read_root_nested_in_instance() -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("maestria-watcher-nested-root-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        let instance = root.join("instance");
        let nested = instance.join("workspace");
        fs::create_dir_all(&nested)?;
        fs::write(nested.join("note.md"), "nested note")?;

        let manifest = InstanceManifest {
            schema_version: 1,
            root: instance,
            read_roots: vec![nested],
            excluded_patterns: Vec::new(),
            embeddings: None,
            ocr: None,
        };
        let observations = scan_manifest(&manifest)?;

        assert_eq!(observations.len(), 1);
        assert!(observations[0].path.ends_with("note.md"));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn scan_excludes_instance_manifest_and_preserves_alias_scope()
    -> Result<(), Box<dyn std::error::Error>> {
        let root =
            env::temp_dir().join(format!("maestria-watcher-instance-alias-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        let instance = root.join("instance");
        fs::create_dir_all(instance.join("system"))?;
        fs::create_dir_all(instance.join("workspace"))?;
        fs::write(instance.join("manifest.txt"), "root=/tmp/secret")?;
        fs::write(instance.join("system").join(WATCH_STATE_FILE), "{}")?;
        fs::write(instance.join("workspace").join("note.md"), "user note")?;

        let manifest = InstanceManifest {
            schema_version: 1,
            root: instance.clone(),
            read_roots: vec![instance.join(".")],
            excluded_patterns: Vec::new(),
            embeddings: None,
            ocr: None,
        };
        let observations = scan_manifest(&manifest)?;

        assert_eq!(observations.len(), 1);
        assert!(observations[0].path.ends_with("workspace/note.md"));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn scan_is_deterministic_and_skips_sensitive_files() -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("maestria-watcher-test-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        fs::write(root.join("note.md"), "note")?;
        fs::write(root.join(".env"), "secret")?;
        let manifest = test_manifest(root.clone());
        let first = scan_manifest(&manifest)?;
        let second = scan_manifest(&manifest)?;
        assert_eq!(
            first.iter().map(|item| &item.path).collect::<Vec<_>>(),
            second.iter().map(|item| &item.path).collect::<Vec<_>>()
        );
        assert_eq!(first.len(), 1);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn scan_respects_gitignore() -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("maestria-watcher-gitignore-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        fs::write(root.join("tracked.md"), "tracked content")?;
        fs::write(root.join("ignored.md"), "ignored content")?;
        fs::write(root.join(".gitignore"), "ignored.md")?;
        let manifest = test_manifest(root.clone());
        let observations = scan_manifest(&manifest)?;
        assert_eq!(observations.len(), 1);
        assert!(
            observations[0].path.ends_with("tracked.md"),
            "only tracked.md should appear, got: {:?}",
            observations[0].path
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn scan_respects_ignore_file() -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("maestria-watcher-ignore-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        fs::write(root.join("ok.md"), "ok")?;
        fs::write(root.join("ignored.md"), "should be ignored")?;
        fs::write(root.join(".ignore"), "ignored.md")?;
        let manifest = test_manifest(root.clone());
        let observations = scan_manifest(&manifest)?;
        assert_eq!(observations.len(), 1);
        assert!(
            observations[0].path.ends_with("ok.md"),
            "only ok.md should appear, got: {:?}",
            observations[0].path
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn state_persistence_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let dir = env::temp_dir().join(format!("maestria-watcher-state-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)?;

        let layout = InstanceLayout::for_root(dir.clone());
        fs::create_dir_all(&layout.system_dir)?;

        let mut state = WatchState::default();
        state
            .files
            .insert("/tmp/a.md".to_string(), "hash1".to_string());
        state
            .removed
            .insert("/tmp/b.md".to_string(), "hash2".to_string());
        state.artifact_ids.insert(
            "/tmp/a.md".to_string(),
            ArtifactIdEntry {
                artifact_id: 42,
                content_hash: "hash1".to_string(),
            },
        );

        persist_state(&layout, &state)?;
        let loaded = load_state(&layout);
        assert_eq!(loaded.files.get("/tmp/a.md"), Some(&"hash1".to_string()));
        assert_eq!(loaded.removed.get("/tmp/b.md"), Some(&"hash2".to_string()));
        assert_eq!(
            loaded.artifact_ids.get("/tmp/a.md").map(|e| e.artifact_id),
            Some(42)
        );

        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[tokio::test]
    async fn scan_once_detects_creation_and_removal() -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("maestria-watcher-e2e-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        let (input_tx, mut input_rx) = mpsc::channel(256);
        let shutdown = CancellationToken::new();

        fs::write(root.join("hello.md"), "hello world")?;

        let manifest = test_manifest(root.clone());
        let state = WatchState::default();
        let scan_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS));
        let mut watcher = Watcher {
            layout: InstanceLayout::for_root(root.clone()),
            manifest,
            input_tx: input_tx.clone(),
            artifact_ids: BTreeMap::new(),
            shutdown: shutdown.clone(),
            state,
            scan_permits,
        };
        watcher.scan_once().await?;
        let detected = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
            .await?
            .ok_or("watcher input channel closed")?;
        assert!(
            matches!(&detected, DomainInput::ArtifactDetected(input) if input.source_path.ends_with("hello.md")),
            "expected ArtifactDetected for hello.md, got {detected:?}"
        );

        // Remove the file and add a different one.
        fs::remove_file(root.join("hello.md"))?;
        fs::write(root.join("other.md"), "other content")?;

        // Scan again.
        watcher.scan_once().await?;

        let mut found_removed = false;
        let mut found_other = false;
        for _ in 0..4 {
            match tokio::time::timeout(Duration::from_secs(5), input_rx.recv()).await {
                Ok(Some(DomainInput::SourceRemoved(input))) => {
                    found_removed = true;
                    assert!(
                        input.source_path.ends_with("hello.md"),
                        "expected SourceRemoved for hello.md, got {input:?}"
                    );
                }
                Ok(Some(DomainInput::ArtifactDetected(input)))
                    if input.source_path.ends_with("other.md") =>
                {
                    found_other = true;
                }
                Ok(None) => break,
                Err(_) => break,
                _ => {}
            }
        }

        assert!(
            found_removed,
            "SourceRemoved was not emitted for removed file"
        );
        assert!(found_other, "ArtifactDetected was not emitted for new file");

        shutdown.cancel();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn changed_file_gets_new_artifact_identity_after_restart()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("maestria-watcher-change-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        let path = root.join("changed.md");
        fs::write(&path, "initial content")?;

        let (input_tx, mut input_rx) = mpsc::channel(256);
        let layout = InstanceLayout::for_root(root.clone());
        let manifest = test_manifest(root.clone());
        let mut first_watcher = Watcher {
            layout: layout.clone(),
            manifest: manifest.clone(),
            input_tx: input_tx.clone(),
            artifact_ids: BTreeMap::new(),
            shutdown: CancellationToken::new(),
            state: WatchState::default(),
            scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        };

        first_watcher.scan_once().await?;
        let first = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
            .await?
            .ok_or("watcher input channel closed")?;
        let first_id = match first {
            DomainInput::ArtifactDetected(input) => input.artifact_id,
            other => return Err(format!("expected first artifact detection, got {other:?}").into()),
        };
        let artifact_ids = first_watcher.artifact_ids.clone();

        fs::write(&path, "updated content")?;
        let mut restarted_watcher = Watcher {
            layout: layout.clone(),
            manifest,
            input_tx,
            artifact_ids,
            shutdown: CancellationToken::new(),
            state: load_state(&layout),
            scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        };

        restarted_watcher.scan_once().await?;
        let changed = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
            .await?
            .ok_or("watcher input channel closed after restart")?;
        let changed_id = match changed {
            DomainInput::ArtifactDetected(input) => input.artifact_id,
            other => {
                return Err(format!(
                    "expected changed artifact detection after restart, got {other:?}"
                )
                .into());
            }
        };

        assert_ne!(
            first_id, changed_id,
            "changed content must create a new artifact version identity after restart"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn state_persistence_survives_restart() -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("maestria-watcher-restart-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        let layout = InstanceLayout::for_root(root.clone());
        fs::create_dir_all(&layout.system_dir)?;

        // Simulate first daemon session.
        fs::write(root.join("survive.md"), "hello")?;
        let (tx, _rx) = mpsc::channel(256);
        let shutdown = CancellationToken::new();
        let manifest = test_manifest(root.clone());
        let state = WatchState::default();
        let scan_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS));
        let mut watcher = Watcher {
            layout: layout.clone(),
            manifest: manifest.clone(),
            input_tx: tx.clone(),
            artifact_ids: BTreeMap::new(),
            shutdown: shutdown.clone(),
            state,
            scan_permits: scan_permits.clone(),
        };

        // Scan so state persists.
        watcher.scan_once().await?;

        // Verify state file was written.
        let state_path = layout.system_dir.join(WATCH_STATE_FILE);
        assert!(state_path.exists(), "watch state must persist after scan");

        // Simulate crash restart by loading persisted state.
        let loaded_state = load_state(&layout);
        assert_eq!(
            loaded_state.files.len(),
            1,
            "should have 1 tracked file after restart load"
        );
        assert!(
            loaded_state.files.values().any(|v| !v.is_empty()),
            "tracked file should have a content hash"
        );

        shutdown.cancel();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn rename_emits_source_removed_for_old_path() -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("maestria-watcher-rename-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        let (input_tx, mut input_rx) = mpsc::channel(256);
        let shutdown = CancellationToken::new();

        // Seed a file and scan once.
        fs::write(root.join("original.md"), "same content")?;
        let manifest = test_manifest(root.clone());
        let state = WatchState::default();
        let scan_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS));
        let mut watcher = Watcher {
            layout: InstanceLayout::for_root(root.clone()),
            manifest: manifest.clone(),
            input_tx: input_tx.clone(),
            artifact_ids: BTreeMap::new(),
            shutdown: shutdown.clone(),
            state,
            scan_permits,
        };

        watcher.scan_once().await?;
        let _ = tokio::time::timeout(Duration::from_secs(5), input_rx.recv()).await;

        // "Rename" by creating a new file with same content and removing old one.
        fs::write(root.join("renamed.md"), "same content")?;
        fs::remove_file(root.join("original.md"))?;

        // Reload persisted state to simulate fresh scan.
        watcher.state = load_state(&watcher.layout);
        watcher.scan_once().await?;

        // Should see SourceRemoved for original.md.
        let mut found = false;
        for _ in 0..4 {
            match tokio::time::timeout(Duration::from_secs(5), input_rx.recv()).await {
                Ok(Some(DomainInput::SourceRemoved(input))) => {
                    if input.source_path.contains("original.md") {
                        found = true;
                    }
                }
                Ok(Some(DomainInput::ArtifactDetected(_))) => {}
                _ => break,
            }
        }

        assert!(found, "rename should emit SourceRemoved for original path");

        shutdown.cancel();
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
