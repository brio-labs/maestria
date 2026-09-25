use anyhow::{Context, Result};
use maestria_core::{InstanceLayout, InstanceManifest};
#[cfg(test)]
use maestria_domain::DomainEvent;
use maestria_domain::DomainInput;
use maestria_storage_sqlite::SqliteStore;
use parking_lot::RwLock;
#[cfg(test)]
use std::path::PathBuf;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;

#[cfg(test)]
use crate::source_identity::source_key;

#[path = "watcher_scan.rs"]
mod watcher_scan;
#[cfg(test)]
use watcher_scan::Observation;
use watcher_scan::scan_manifest;
#[path = "watcher_state.rs"]
mod watcher_state;
#[cfg(test)]
use watcher_state::WATCH_STATE_FILE;
use watcher_state::{ArtifactIdEntry, WatchState, load_state, persist_state, unix_time_millis};
#[path = "watcher_freshness.rs"]
mod watcher_freshness;
pub(crate) use watcher_freshness::{
    fresh_source_paths, fresh_source_paths_bounded, is_internal_source_path,
};
pub(crate) use watcher_state::status;
#[path = "watcher_receipts.rs"]
mod watcher_receipts;
#[cfg(test)]
use watcher_receipts::pending_removal_key;
use watcher_receipts::{
    PendingDelivery, PendingDeliveryStatus, ReceiptTracking, pending_delivery_key,
};
#[path = "watcher_projection.rs"]
mod watcher_projection;
const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum number of concurrent scan operations. Prevents unbounded I/O
/// when the manifest contains many read roots.
const MAX_CONCURRENT_SCANS: usize = 4;

pub(crate) fn spawn(
    layout: InstanceLayout,
    manifest: Arc<RwLock<InstanceManifest>>,
    input_tx: mpsc::Sender<DomainInput>,
    artifact_ids: BTreeMap<String, (maestria_domain::ArtifactId, String)>,
    shutdown: CancellationToken,
) -> JoinHandle<Result<()>> {
    tokio::spawn(async move {
        let event_log = SqliteStore::open_read_only(&layout.database_path).with_context(|| {
            format!("open watcher event log {}", layout.database_path.display())
        })?;
        let mut state = load_state(&layout);
        // The event log is authoritative for active source identities. Drop
        // accepted-state rows whose ParserStarted marker was subsequently
        // revoked before the watcher could persist its matching tombstone.
        state.files.retain(|key, hash| {
            artifact_ids
                .get(key)
                .is_some_and(|(_, accepted_hash)| accepted_hash == hash)
        });
        state.artifact_ids = artifact_ids
            .iter()
            .map(|(key, (artifact_id, content_hash))| {
                (
                    key.clone(),
                    ArtifactIdEntry {
                        artifact_id: artifact_id.value(),
                        content_hash: content_hash.clone(),
                    },
                )
            })
            .collect();
        let scan_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS));
        let watcher = Watcher {
            layout,
            manifest,
            input_tx,
            artifact_ids: artifact_ids.into_iter().collect(),
            shutdown,
            state,
            pending: BTreeMap::new(),
            receipts: ReceiptTracking::new(event_log),
            scan_permits,
        };
        watcher.run().await
    })
}

struct Watcher {
    layout: InstanceLayout,
    manifest: Arc<RwLock<InstanceManifest>>,
    input_tx: mpsc::Sender<DomainInput>,
    artifact_ids: BTreeMap<String, (maestria_domain::ArtifactId, String)>,
    shutdown: CancellationToken,
    state: WatchState,
    /// Delivery identity remains pending until its ParserStarted marker is
    /// visible in the durable event log.
    pending: BTreeMap<String, PendingDelivery>,
    receipts: ReceiptTracking,
    scan_permits: Arc<Semaphore>,
}

impl Watcher {
    async fn run(mut self) -> Result<()> {
        let mut ticks = interval(WATCH_INTERVAL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                _ = ticks.tick() => {
                    self.state.scanning = true;
                    self.state.pending_files = self.pending_file_count();
                    if let Err(error) = persist_state(&self.layout, &self.state) {
                        tracing::warn!(%error, "failed to persist indexing progress status");
                    }
                    if let Err(error) = self.scan_once().await {
                        self.state.scanning = false;
                        self.state.pending_files = self.pending_file_count();
                        self.state.last_error = Some(error.to_string());
                        if let Err(persist_error) = persist_state(&self.layout, &self.state) {
                            tracing::warn!(%persist_error, "failed to persist indexing error status");
                        }
                        tracing::warn!(%error, "continuous ingestion scan failed");
                    }
                }
            }
        }
        persist_state(&self.layout, &self.state)
            .with_context(|| "persist continuous ingestion state on shutdown")
    }

    async fn scan_once(&mut self) -> Result<()> {
        let permits = self.scan_permits.clone();
        let _permit = permits.acquire().await.context("acquire scan permit")?;

        let previous_artifact_ids = self.state.artifact_ids.clone();
        let confirmed = self.phase_confirm_deliveries()?;
        let manifest = self.manifest.read().clone();
        let (observations, signatures) =
            scan_manifest(&manifest, &self.state.signatures, &self.state.files)?;
        let mut current = self.phase_detect_additions(observations).await?;

        // Unchanged accepted files produced no observation; retain their hash
        // so source removals can still be reconciled.
        for key in signatures.keys() {
            if !current.contains_key(key)
                && let Some(hash) = self.state.files.get(key)
            {
                current.insert(key.clone(), hash.clone());
            }
        }
        self.state.signatures = signatures;

        // An accepted delivery whose source disappeared or changed before this
        // scan still needs a durable revocation.
        for (source_path, delivery) in confirmed {
            let delivery_key = pending_delivery_key(&source_path, delivery.artifact_id.value());
            if current.get(&source_path) == Some(&delivery.content_hash) {
                let entry = ArtifactIdEntry {
                    artifact_id: delivery.artifact_id.value(),
                    content_hash: delivery.content_hash.clone(),
                };
                self.artifact_ids.insert(
                    source_path.clone(),
                    (delivery.artifact_id, delivery.content_hash.clone()),
                );
                self.state.artifact_ids.insert(source_path.clone(), entry);
                self.pending.remove(&delivery_key);
                continue;
            }
            self.queue_source_removal(
                &source_path,
                &ArtifactIdEntry {
                    artifact_id: delivery.artifact_id.value(),
                    content_hash: delivery.content_hash.clone(),
                },
            );
            self.pending.remove(&delivery_key);
        }

        // Preserve enqueued deliveries until their durable marker is either
        // accepted or revoked; a later edit must not orphan an in-flight
        // artifact whose parser starts after this scan.
        let obsolete_enqueued: Vec<_> = self
            .pending
            .values()
            .filter(|delivery| {
                delivery.status == PendingDeliveryStatus::Enqueued
                    && current.get(&delivery.source_path) != Some(&delivery.content_hash)
            })
            .cloned()
            .collect();
        for delivery in obsolete_enqueued {
            self.queue_source_removal(
                &delivery.source_path,
                &ArtifactIdEntry {
                    artifact_id: delivery.artifact_id.value(),
                    content_hash: delivery.content_hash,
                },
            );
        }
        self.pending.retain(|_, delivery| {
            delivery.status == PendingDeliveryStatus::Enqueued
                || current.get(&delivery.source_path) == Some(&delivery.content_hash)
        });
        let pending_files = self.pending_file_count_for(&current);

        let previous_files = std::mem::replace(&mut self.state.files, current);
        self.phase_detect_removals(previous_files, previous_artifact_ids)
            .await?;

        // Newly enqueued or backpressured observations are not indexed until
        // their ParserStarted marker is durable.
        self.retain_accepted_files();
        self.state
            .artifact_ids
            .retain(|key, entry| self.state.files.get(key) == Some(&entry.content_hash));
        self.phase_process_pending_removals()?;
        self.state.scanning = false;
        self.state.last_error = None;
        self.state.pending_files = pending_files;
        self.state.last_scan_unix_ms = Some(unix_time_millis());
        persist_state(&self.layout, &self.state)
    }
}

#[cfg(test)]
fn test_manifest(root: PathBuf) -> Result<InstanceManifest, Box<dyn std::error::Error>> {
    Ok(InstanceManifest {
        schema_version: 2,
        realm_id: maestria_test_support::realm_id(10)?,
        root: root.clone(),
        read_roots: vec![root],
        excluded_patterns: vec![".env".to_string()],
        embeddings: None,
        ocr: None,
        visual: None,
        sparse: None,
    })
}

#[cfg(test)]
fn test_receipts() -> Result<ReceiptTracking, Box<dyn std::error::Error>> {
    Ok(ReceiptTracking::new(SqliteStore::in_memory()?))
}

#[cfg(test)]
#[path = "watcher_tests/mod.rs"]
mod watcher_tests;

#[cfg(test)]
#[path = "watcher_removal_tests.rs"]
mod watcher_removal_tests;
