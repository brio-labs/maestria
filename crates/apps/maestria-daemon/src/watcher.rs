use anyhow::{Context, Result};
use maestria_core::{InstanceLayout, InstanceManifest};
#[cfg(test)]
use maestria_domain::DomainEvent;
use maestria_domain::DomainInput;
use maestria_runtime::RuntimeHandle;
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

use crate::search_executor::SearchRuntime;

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
    fresh_source_paths, fresh_source_paths_bounded, fresh_source_paths_rehashed,
    is_internal_source_path,
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
/// Bound unconfirmed observations so durable interactive commands do not wait
/// behind an entire scan's event-log writes and parse effects.
const MAX_ENQUEUED_DELIVERIES: usize = 8;

pub(crate) fn spawn(
    layout: InstanceLayout,
    manifest: Arc<RwLock<InstanceManifest>>,
    input_tx: mpsc::Sender<DomainInput>,
    artifact_ids: BTreeMap<String, (maestria_domain::ArtifactId, String)>,
    shutdown: CancellationToken,
    search_runtime: SearchRuntime,
    runtime_handle: RuntimeHandle,
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
        watcher.run(search_runtime, runtime_handle).await
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
    async fn run(
        mut self,
        search_runtime: SearchRuntime,
        runtime_handle: RuntimeHandle,
    ) -> Result<()> {
        let mut snapshot_refresh = SnapshotRefreshState::new(search_runtime, runtime_handle);
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
                    if let Err(error) = self
                        .scan_once_with_snapshot_refresh(&mut snapshot_refresh)
                        .await
                    {
                        self.state.scanning = !snapshot_refresh
                            .is_ready_for_current(
                                !self.pending.is_empty(),
                                !self.state.pending_removals.is_empty(),
                            )
                            .await;
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

    #[cfg(test)]
    async fn scan_once(&mut self) -> Result<()> {
        self.scan_once_inner(None).await
    }

    async fn scan_once_with_snapshot_refresh(
        &mut self,
        refresh: &mut SnapshotRefreshState,
    ) -> Result<()> {
        self.scan_once_inner(Some(refresh)).await
    }

    async fn scan_once_inner(&mut self, refresh: Option<&mut SnapshotRefreshState>) -> Result<()> {
        let permits = self.scan_permits.clone();
        let permit = permits.acquire().await.context("acquire scan permit")?;
        let previous_artifact_ids = self.state.artifact_ids.clone();
        let confirmed = self.phase_confirm_deliveries()?;
        let manifest = self.manifest.read().clone();
        let (observations, signatures) =
            scan_manifest(&manifest, &self.state.signatures, &self.state.files)?;
        let mut current = self
            .phase_detect_additions(observations, confirmed.len())
            .await?;

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
        drop(permit);
        let readiness = match refresh {
            Some(refresh) => {
                refresh
                    .prepare_if_ready(
                        !self.pending.is_empty(),
                        !self.state.pending_removals.is_empty(),
                    )
                    .await
            }
            None => SnapshotReadiness::Ready,
        };
        self.state.scanning = !matches!(&readiness, SnapshotReadiness::Ready);
        self.state.last_error = match readiness {
            SnapshotReadiness::Ready | SnapshotReadiness::Pending => None,
            SnapshotReadiness::Failed(error) => Some(error),
        };
        self.state.pending_files = pending_files;
        self.state.last_scan_unix_ms = Some(unix_time_millis());
        persist_state(&self.layout, &self.state)
    }
}

enum SnapshotReadiness {
    Ready,
    Pending,
    Failed(String),
}

struct SnapshotRefreshState {
    search_runtime: SearchRuntime,
    runtime_handle: RuntimeHandle,
    attempted_revision: Option<i64>,
    failure: Option<(i64, String)>,
}

impl SnapshotRefreshState {
    fn new(search_runtime: SearchRuntime, runtime_handle: RuntimeHandle) -> Self {
        Self {
            search_runtime,
            runtime_handle,
            attempted_revision: None,
            failure: None,
        }
    }

    async fn is_ready_for_current(&self, pending_deliveries: bool, pending_removals: bool) -> bool {
        if pending_deliveries || pending_removals || self.runtime_handle.has_pending_parsers().await
        {
            return false;
        }
        let Ok(revision) = self.search_runtime.searchable_source_revision() else {
            return false;
        };
        self.search_runtime
            .interactive_snapshot_is_current(revision)
    }

    async fn prepare_if_ready(
        &mut self,
        pending_deliveries: bool,
        pending_removals: bool,
    ) -> SnapshotReadiness {
        let current_revision = match self.search_runtime.searchable_source_revision() {
            Ok(revision) => revision,
            Err(error) => {
                return SnapshotReadiness::Failed(format!(
                    "read searchable source revision: {error:#}"
                ));
            }
        };
        if self
            .failure
            .as_ref()
            .is_some_and(|(revision, _)| *revision != current_revision)
        {
            self.failure = None;
        }
        let cached_current = self
            .search_runtime
            .interactive_snapshot_is_current(current_revision);
        if cached_current {
            self.failure = None;
        } else if let Some((revision, error)) = &self.failure
            && *revision == current_revision
        {
            return SnapshotReadiness::Failed(error.clone());
        }

        if pending_deliveries || pending_removals || self.runtime_handle.has_pending_parsers().await
        {
            return SnapshotReadiness::Pending;
        }
        if cached_current {
            let latest_revision = match self.search_runtime.searchable_source_revision() {
                Ok(revision) => revision,
                Err(error) => {
                    return self.record_failure(
                        current_revision,
                        format!("recheck searchable source revision: {error:#}"),
                    );
                }
            };
            if latest_revision != current_revision
                || !self
                    .search_runtime
                    .interactive_snapshot_is_current(latest_revision)
            {
                return SnapshotReadiness::Pending;
            }
            return SnapshotReadiness::Ready;
        }
        if self.attempted_revision == Some(current_revision) {
            return self
                .failure
                .as_ref()
                .filter(|(revision, _)| *revision == current_revision)
                .map_or(SnapshotReadiness::Pending, |(_, error)| {
                    SnapshotReadiness::Failed(error.clone())
                });
        }

        self.attempted_revision = Some(current_revision);
        let search_runtime = self.search_runtime.clone();
        let preparation =
            tokio::task::spawn_blocking(move || search_runtime.prepare_interactive_snapshot())
                .await;
        let (prepared_revision, ready) = match preparation {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(error)) => {
                return self.record_failure(
                    current_revision,
                    format!("prepare interactive source snapshot: {error:#}"),
                );
            }
            Err(error) => {
                return self.record_failure(
                    current_revision,
                    format!("join interactive source snapshot preparation: {error}"),
                );
            }
        };
        if !ready {
            if prepared_revision == current_revision {
                return self.record_failure(
                    current_revision,
                    "interactive source snapshot cache did not reach the current revision"
                        .to_owned(),
                );
            }
            self.failure = None;
            return SnapshotReadiness::Pending;
        }

        let latest_revision = match self.search_runtime.searchable_source_revision() {
            Ok(revision) => revision,
            Err(error) => {
                return self.record_failure(
                    current_revision,
                    format!("recheck searchable source revision: {error:#}"),
                );
            }
        };
        if latest_revision != prepared_revision
            || !self
                .search_runtime
                .interactive_snapshot_is_current(latest_revision)
            || self.runtime_handle.has_pending_parsers().await
        {
            self.failure = None;
            return SnapshotReadiness::Pending;
        }
        self.failure = None;
        SnapshotReadiness::Ready
    }

    fn record_failure(&mut self, revision: i64, error: String) -> SnapshotReadiness {
        tracing::warn!(
            revision,
            error = %error,
            "interactive source snapshot preparation failed"
        );
        self.failure = Some((revision, error.clone()));
        SnapshotReadiness::Failed(error)
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
