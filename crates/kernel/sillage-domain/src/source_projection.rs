use crate::events::{DomainEvent, DomainEventEnvelope};
use crate::ids::{ArtifactId, ArtifactVersionId};
use crate::search::ContentHash;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
use crate::ids::{BlobId, EventId, StructureNodeId};

/// Currently active indexed versions, keyed by canonical source path.
pub type ActiveSourceVersions = BTreeMap<PathBuf, (ArtifactId, ArtifactVersionId, ContentHash)>;

/// Replay state for source events. Keep historical captures so reapproving
/// identical bytes restores their content-addressed version after a stale event.
#[derive(Clone, Default)]
pub struct SourceProjection {
    sources: Arc<ActiveSourceVersions>,
    path_by_artifact: BTreeMap<ArtifactId, String>,
    captured_versions: BTreeMap<ArtifactId, BTreeMap<ContentHash, ArtifactVersionId>>,
}

impl SourceProjection {
    pub fn sources(&self) -> Arc<ActiveSourceVersions> {
        self.sources.clone()
    }

    /// Apply all source-version events since the previous batch in ID order;
    /// unrelated audit event IDs may be skipped.
    pub fn apply(&mut self, events: &[DomainEventEnvelope]) {
        project_source_events(
            Arc::make_mut(&mut self.sources),
            &mut self.path_by_artifact,
            &mut self.captured_versions,
            events,
        );
    }
}

/// Projects the currently active source versions from the append-only event
/// log, keyed by canonical source path.
///
/// `ParserStarted` records the source path, initially with a placeholder
/// version derived from the artifact id. `DocumentTreeCaptured` replaces it
/// with the content-addressed version. If an identical source is reapproved
/// after `SourceBecameStale`, the previous captured version remains valid and
/// must be restored even when parsing emits no duplicate tree event. Consumers
/// share this projection so stale versions never surface in retrieval.
pub fn active_source_versions(events: &[DomainEventEnvelope]) -> ActiveSourceVersions {
    let mut active = BTreeMap::new();
    project_source_events(
        &mut active,
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        events,
    );
    active
}

fn project_source_events(
    active: &mut ActiveSourceVersions,
    path_by_artifact: &mut BTreeMap<ArtifactId, String>,
    captured_versions: &mut BTreeMap<ArtifactId, BTreeMap<ContentHash, ArtifactVersionId>>,
    events: &[DomainEventEnvelope],
) {
    for envelope in events {
        match &envelope.event {
            DomainEvent::ParserStarted {
                artifact_id,
                source_path,
                content_hash,
                ..
            } => {
                path_by_artifact.insert(*artifact_id, source_path.clone());
                let version = match captured_versions
                    .get(artifact_id)
                    .and_then(|by_hash| by_hash.get(content_hash))
                {
                    Some(version) => *version,
                    None => ArtifactVersionId::new(artifact_id.value()),
                };
                active.insert(
                    PathBuf::from(source_path),
                    (*artifact_id, version, content_hash.clone()),
                );
            }
            DomainEvent::DocumentTreeCaptured {
                artifact_id,
                artifact_version_id,
                content_hash,
                ..
            } => {
                captured_versions
                    .entry(*artifact_id)
                    .or_default()
                    .insert(content_hash.clone(), *artifact_version_id);
                if let Some(path) = path_by_artifact.get(artifact_id)
                    && let Some(entry) = active.get_mut(Path::new(path))
                    && entry.0 == *artifact_id
                    && entry.2 == *content_hash
                {
                    entry.1 = *artifact_version_id;
                }
            }
            DomainEvent::SourceBecameStale {
                artifact_id,
                source_path,
                content_hash,
            } => {
                let path = PathBuf::from(source_path);
                if active
                    .get(&path)
                    .is_some_and(|(active_id, _, active_hash)| {
                        active_id == artifact_id && active_hash == content_hash
                    })
                {
                    active.remove(&path);
                }
                path_by_artifact.remove(artifact_id);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod source_projection_tests {
    use super::*;

    #[test]
    fn incremental_replay_matches_full_history_across_reapproval_and_replacement()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = "/approved/source.md".to_string();
        let first_hash = ContentHash::new(crate::provenance::content_hash(b"first bytes"))?;
        let second_hash = ContentHash::new(crate::provenance::content_hash(b"second bytes"))?;
        let first = ArtifactId::new(7);
        let second = ArtifactId::new(8);
        let first_version = ArtifactVersionId::new(71);
        let second_version = ArtifactVersionId::new(81);
        let started = |id: u64, artifact_id, content_hash: ContentHash| DomainEventEnvelope {
            id: EventId::new(id),
            event: DomainEvent::ParserStarted {
                artifact_id,
                title: "source.md".to_string(),
                source_path: path.clone(),
                content_hash,
                blob_id: BlobId::new(id),
            },
        };
        let stale = |id: u64, artifact_id, content_hash: ContentHash| DomainEventEnvelope {
            id: EventId::new(id),
            event: DomainEvent::SourceBecameStale {
                artifact_id,
                source_path: path.clone(),
                content_hash,
            },
        };
        let captured = |id: u64, artifact_id, artifact_version_id, content_hash: ContentHash| {
            DomainEventEnvelope {
                id: EventId::new(id),
                event: DomainEvent::DocumentTreeCaptured {
                    artifact_id,
                    artifact_version_id,
                    content_hash,
                    root_id: StructureNodeId::new(id),
                    nodes: Vec::new(),
                },
            }
        };
        let history = [
            started(1, first, first_hash.clone()),
            captured(2, first, first_version, first_hash.clone()),
            stale(3, first, first_hash.clone()),
            started(4, first, first_hash.clone()),
            started(5, second, second_hash.clone()),
            stale(6, first, first_hash.clone()),
            captured(7, second, second_version, second_hash.clone()),
            stale(8, second, second_hash),
            started(9, first, first_hash.clone()),
        ];
        let mut projection = SourceProjection::default();
        let mut cursor = 0;
        let mut retained_before_stale = None;
        for boundary in [2, 3, 4, 6, 8, 9] {
            projection.apply(&history[cursor..boundary]);
            assert_eq!(
                projection.sources().as_ref(),
                &active_source_versions(&history[..boundary]),
                "incremental source replay diverged at event {boundary}"
            );
            if boundary == 2 {
                retained_before_stale = Some(projection.sources());
            }
            if boundary == 3 {
                assert!(projection.sources().get(Path::new(&path)).is_none());
                assert_eq!(
                    retained_before_stale.as_ref().and_then(
                        |sources: &Arc<ActiveSourceVersions>| {
                            sources
                                .get(Path::new(&path))
                                .map(|(_, version, _)| *version)
                        }
                    ),
                    Some(first_version),
                    "a concurrent old snapshot must remain immutable until its own revision check"
                );
            }
            cursor = boundary;
        }
        assert_eq!(
            projection.sources().get(Path::new(&path)),
            Some(&(first, first_version, first_hash)),
            "identical bytes must restore the original captured version"
        );
        Ok(())
    }
}
