use anyhow::Result;
use maestria_core::build_artifact_detected_input;
use std::collections::{BTreeMap, BTreeSet};
use tokio::sync::mpsc;

use crate::source_identity::source_key;

use super::watcher_receipts::pending_removal_key;

use super::watcher_scan::Observation;

use super::{
    ArtifactIdEntry, DomainInput, PendingDelivery, PendingDeliveryStatus, Watcher,
    pending_delivery_key,
};

impl Watcher {
    pub(super) fn pending_file_count(&self) -> usize {
        self.pending
            .values()
            .map(|delivery| delivery.source_path.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub(super) fn pending_file_count_for(&self, current: &BTreeMap<String, String>) -> usize {
        self.pending
            .values()
            .filter(|delivery| current.get(&delivery.source_path) == Some(&delivery.content_hash))
            .map(|delivery| delivery.source_path.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub(super) fn retain_accepted_files(&mut self) {
        let pending_hashes: BTreeSet<_> = self
            .pending
            .values()
            .map(|delivery| {
                (
                    delivery.source_path.as_str(),
                    delivery.content_hash.as_str(),
                )
            })
            .collect();
        self.state
            .files
            .retain(|key, hash| !pending_hashes.contains(&(key.as_str(), hash.as_str())));
    }

    fn find_pending_delivery(
        &self,
        source_path: &str,
        content_hash: &str,
    ) -> Option<(String, PendingDelivery)> {
        let prefix = format!("{source_path}\0");
        self.pending
            .range(prefix.clone()..)
            .take_while(|(delivery_key, _)| delivery_key.starts_with(&prefix))
            .find(|(_, delivery)| {
                delivery.source_path == source_path && delivery.content_hash == content_hash
            })
            .map(|(delivery_key, delivery)| (delivery_key.clone(), delivery.clone()))
    }

    fn remove_pending_deliveries(&mut self, source_path: &str, content_hash: &str) {
        let prefix = format!("{source_path}\0");
        let keys = self
            .pending
            .range(prefix.clone()..)
            .take_while(|(delivery_key, _)| delivery_key.starts_with(&prefix))
            .filter(|(_, delivery)| delivery.content_hash == content_hash)
            .map(|(delivery_key, _)| delivery_key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            self.pending.remove(&key);
        }
    }

    pub(super) async fn phase_detect_additions(
        &mut self,
        observations: Vec<Observation>,
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
                self.remove_pending_deliveries(&key, &observation.hash);
                if self.state.removed.get(&key) == Some(&observation.hash) {
                    self.state.removed.remove(&key);
                }
                continue;
            }

            let existing = self.find_pending_delivery(&key, &observation.hash);
            if existing
                .as_ref()
                .is_some_and(|(_, pending)| pending.status == PendingDeliveryStatus::Enqueued)
            {
                continue;
            }
            let input = match build_artifact_detected_input(
                &observation.path,
                observation.bytes,
                observation.hash.clone(),
            ) {
                Ok(input) => input,
                Err(error) => {
                    tracing::warn!(path = %key, error = %error, "watcher observed invalid artifact input; skipping detection");
                    continue;
                }
            };
            let artifact_id = match &input {
                DomainInput::ArtifactDetected(detected) => detected.artifact_id,
                _ => anyhow::bail!("artifact input builder returned a non-detection input"),
            };
            let delivery_key = match existing {
                Some((delivery_key, _)) => delivery_key,
                None => pending_delivery_key(&key, artifact_id.value()),
            };

            let status = match self.input_tx.try_send(input) {
                Ok(()) => PendingDeliveryStatus::Enqueued,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::debug!("watcher input channel full — deferring artifact detection");
                    PendingDeliveryStatus::Deferred
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(anyhow::anyhow!(
                        "submit watched artifact: input channel closed"
                    ));
                }
            };
            self.pending.insert(
                delivery_key,
                PendingDelivery {
                    source_path: key.clone(),
                    artifact_id,
                    content_hash: observation.hash.clone(),
                    status,
                },
            );
            if self.state.removed.get(&key) == Some(&observation.hash) {
                self.state.removed.remove(&key);
            }
        }

        Ok(current)
    }

    pub(super) async fn phase_detect_removals(
        &mut self,
        previous_files: BTreeMap<String, String>,
        previous_artifact_ids: BTreeMap<String, ArtifactIdEntry>,
    ) -> Result<()> {
        let hash_index: BTreeMap<String, String> = self
            .state
            .files
            .iter()
            .map(|(key, hash)| (hash.clone(), key.clone()))
            .collect();

        for (previous_path, previous_hash) in &previous_files {
            if self.state.files.get(previous_path) == Some(previous_hash) {
                continue;
            }
            if let Some(new_path) = hash_index.get(previous_hash)
                && new_path != previous_path
            {
                tracing::info!(
                    from = %previous_path,
                    to = %new_path,
                    "watcher detected file rename"
                );
            }
        }

        // A path whose content changed is a removal of the old artifact just
        // as surely as a path that disappeared. Keep the old identity in the
        // durable retry queue until SourceBecameStale is visible.
        for (source_path, entry) in previous_artifact_ids {
            if self.state.files.get(&source_path) == Some(&entry.content_hash) {
                continue;
            }
            self.queue_source_removal(&source_path, &entry);
        }
        Ok(())
    }

    pub(super) fn queue_source_removal(&mut self, source_path: &str, entry: &ArtifactIdEntry) {
        let key = pending_removal_key(source_path, entry.artifact_id);
        self.state.pending_removals.entry(key).or_insert_with(|| {
            super::watcher_state::PendingRemovalEntry {
                source_path: source_path.to_owned(),
                artifact_id: entry.artifact_id,
                content_hash: entry.content_hash.clone(),
            }
        });
    }
}
