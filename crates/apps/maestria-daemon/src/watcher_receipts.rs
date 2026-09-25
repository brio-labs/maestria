use anyhow::Result;
use maestria_domain::{DomainEvent, DomainInput, SourceRemoved};
use maestria_ports::{EventFilter, EventLog};
use maestria_storage_sqlite::SqliteStore;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use tokio::sync::mpsc;

use crate::source_identity::source_key;

use super::{Watcher, watcher_state::PendingRemovalEntry};

const MAX_ACCEPTANCE_PROBES_PER_SCAN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingDeliveryStatus {
    /// The input was accepted by the bounded channel, but the runtime has not
    /// durably reported acceptance yet.
    Enqueued,
    /// The input could not be queued because the bounded channel was full.
    Deferred,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceReceipt {
    Unseen,
    Active,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingDelivery {
    pub(super) source_path: String,
    pub(super) artifact_id: maestria_domain::ArtifactId,
    pub(super) content_hash: String,
    pub(super) status: PendingDeliveryStatus,
}

pub(super) struct ReceiptTracking {
    pub(super) event_log: SqliteStore,
    pending_removal_deliveries: BTreeSet<String>,
    acceptance_cursor: Option<String>,
    removal_cursor: Option<String>,
}

impl ReceiptTracking {
    pub(super) fn new(event_log: SqliteStore) -> Self {
        Self {
            event_log,
            pending_removal_deliveries: BTreeSet::new(),
            acceptance_cursor: None,
            removal_cursor: None,
        }
    }
}

impl Watcher {
    /// Confirm only a bounded, rotating batch of enqueued detections. Each
    /// probe is restricted to that artifact's event-log partition.
    pub(super) fn phase_confirm_deliveries(&mut self) -> Result<Vec<(String, PendingDelivery)>> {
        let keys = bounded_keys(&self.pending, &mut self.receipts.acceptance_cursor);
        let mut confirmed = Vec::new();
        for key in keys {
            let Some(delivery) = self.pending.get(&key).cloned() else {
                continue;
            };
            if delivery.status != PendingDeliveryStatus::Enqueued
                || self.source_receipt_state(
                    delivery.artifact_id,
                    &delivery.source_path,
                    &delivery.content_hash,
                )? != SourceReceipt::Active
            {
                continue;
            }
            confirmed.push((delivery.source_path.clone(), delivery));
        }
        Ok(confirmed)
    }

    fn source_receipt_state(
        &self,
        artifact_id: maestria_domain::ArtifactId,
        source_path: &str,
        content_hash: &str,
    ) -> Result<SourceReceipt> {
        let events = self.receipts.event_log.scan(EventFilter {
            artifact_id: Some(artifact_id),
        })?;
        let mut receipt = SourceReceipt::Unseen;
        for envelope in events {
            match &envelope.event {
                DomainEvent::ParserStarted {
                    artifact_id: event_artifact,
                    source_path: event_path,
                    content_hash: event_hash,
                    ..
                } if *event_artifact == artifact_id
                    && source_key(Path::new(event_path)) == source_path
                    && event_hash.as_str() == content_hash =>
                {
                    receipt = SourceReceipt::Active;
                }
                DomainEvent::SourceBecameStale {
                    artifact_id: event_artifact,
                    source_path: event_path,
                    content_hash: event_hash,
                } if *event_artifact == artifact_id
                    && source_key(Path::new(event_path)) == source_path
                    && event_hash.as_str() == content_hash =>
                {
                    receipt = SourceReceipt::Stale;
                }
                _ => {}
            }
        }
        Ok(receipt)
    }

    /// Reconcile a bounded, rotating batch of revocations. Queue membership is
    /// durable, while enqueue status is intentionally in-memory so restarts
    /// retry any SourceRemoved that was not durably applied.
    pub(super) fn phase_process_pending_removals(&mut self) -> Result<()> {
        let keys = bounded_keys(
            &self.state.pending_removals,
            &mut self.receipts.removal_cursor,
        );
        for key in keys {
            let Some(removal) = self.state.pending_removals.get(&key).cloned() else {
                continue;
            };
            let receipt = self.source_receipt_state(
                maestria_domain::ArtifactId::new(removal.artifact_id),
                &removal.source_path,
                &removal.content_hash,
            )?;
            if receipt == SourceReceipt::Stale {
                self.state.pending_removals.remove(&key);
                self.receipts.pending_removal_deliveries.remove(&key);
                self.state
                    .removed
                    .insert(removal.source_path.clone(), removal.content_hash.clone());
                if self
                    .state
                    .files
                    .get(&removal.source_path)
                    .is_some_and(|hash| hash == &removal.content_hash)
                {
                    self.state.files.remove(&removal.source_path);
                }
                if self
                    .state
                    .artifact_ids
                    .get(&removal.source_path)
                    .is_some_and(|entry| {
                        entry.artifact_id == removal.artifact_id
                            && entry.content_hash == removal.content_hash
                    })
                {
                    self.state.artifact_ids.remove(&removal.source_path);
                }
                if self
                    .artifact_ids
                    .get(&removal.source_path)
                    .is_some_and(|(artifact_id, _)| artifact_id.value() == removal.artifact_id)
                {
                    self.artifact_ids.remove(&removal.source_path);
                }
                continue;
            }
            if receipt != SourceReceipt::Active {
                continue;
            }
            if self.receipts.pending_removal_deliveries.contains(&key) {
                continue;
            }
            if self.emit_source_removed(&removal)? {
                self.receipts.pending_removal_deliveries.insert(key);
            } else {
                tracing::debug!(
                    source_path = %removal.source_path,
                    "deferring SourceRemoved emission (channel full)"
                );
            }
        }
        Ok(())
    }

    fn emit_source_removed(&self, removal: &PendingRemovalEntry) -> Result<bool> {
        let Ok(content_hash) = maestria_domain::ContentHash::new(removal.content_hash.clone())
        else {
            tracing::warn!(
                source_path = %removal.source_path,
                "watcher observed an invalid content hash; skipping removal"
            );
            return Ok(false);
        };

        match self
            .input_tx
            .try_send(DomainInput::SourceRemoved(SourceRemoved {
                artifact_id: maestria_domain::ArtifactId::new(removal.artifact_id),
                source_path: removal.source_path.clone(),
                content_hash,
            })) {
            Ok(()) => Ok(true),
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::debug!(
                    source_path = %removal.source_path,
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

pub(super) fn pending_delivery_key(source_path: &str, artifact_id: u64) -> String {
    format!("{source_path}\0{artifact_id}")
}

pub(super) fn pending_removal_key(source_path: &str, artifact_id: u64) -> String {
    format!("{source_path}\0{artifact_id}")
}

fn bounded_keys<V>(map: &BTreeMap<String, V>, cursor: &mut Option<String>) -> Vec<String> {
    use std::ops::Bound::{Excluded, Unbounded};

    let mut keys = Vec::with_capacity(MAX_ACCEPTANCE_PROBES_PER_SCAN.min(map.len()));
    if let Some(last) = cursor.as_ref() {
        keys.extend(
            map.range((Excluded(last.clone()), Unbounded))
                .take(MAX_ACCEPTANCE_PROBES_PER_SCAN)
                .map(|(key, _)| key.clone()),
        );
        if keys.len() < MAX_ACCEPTANCE_PROBES_PER_SCAN {
            keys.extend(
                map.range(..=last.clone())
                    .take(MAX_ACCEPTANCE_PROBES_PER_SCAN - keys.len())
                    .map(|(key, _)| key.clone()),
            );
        }
    } else {
        keys.extend(map.keys().take(MAX_ACCEPTANCE_PROBES_PER_SCAN).cloned());
    }
    *cursor = keys.last().cloned();
    keys
}
