use anyhow::{Context, Result};
use maestria_core::{InstanceLayout, InstanceManifest, artifact_id_for};
use maestria_domain::{ArtifactDetected, DomainInput, SourceRemoved};
#[cfg(test)]
use std::path::PathBuf;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;

#[path = "watcher_scan.rs"]
mod watcher_scan;
use watcher_scan::{Observation, scan_manifest, source_key};
#[path = "watcher_state.rs"]
mod watcher_state;
#[cfg(test)]
use watcher_state::WATCH_STATE_FILE;
use watcher_state::{ArtifactIdEntry, WatchState, load_state, persist_state};

const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum number of concurrent scan operations. Prevents unbounded I/O
/// when the manifest contains many read roots.
const MAX_CONCURRENT_SCANS: usize = 4;

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
            let Ok(content_hash) = maestria_domain::ContentHash::new(observation.hash.clone())
            else {
                tracing::warn!(
                    path = %key,
                    "watcher observed an invalid content hash; skipping detection"
                );
                continue;
            };

            match self
                .input_tx
                .try_send(DomainInput::ArtifactDetected(ArtifactDetected {
                    artifact_id,
                    title,
                    source_path: key.clone(),
                    source_bytes: observation.bytes.clone(),
                    content_hash,
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

        let Ok(content_hash) = maestria_domain::ContentHash::new(entry_hash.clone()) else {
            tracing::warn!(
                source_path = %prev_key,
                "watcher observed an invalid content hash; skipping removal"
            );
            return Ok(false);
        };

        match self
            .input_tx
            .try_send(DomainInput::SourceRemoved(SourceRemoved {
                artifact_id: maestria_domain::ArtifactId::new(*aid_val),
                source_path: prev_key.to_string(),
                content_hash,
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
#[path = "watcher_tests/mod.rs"]
mod watcher_tests;

#[cfg(test)]
#[path = "watcher_removal_tests.rs"]
mod watcher_removal_tests;
