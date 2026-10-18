use super::*;
use sillage_domain::{ArtifactId, BlobId, DomainEvent, DomainEventEnvelope, EventId};

#[test]
fn preview_revision_refresh_keeps_unrelated_source_and_denies_revoked_source()
-> Result<(), Box<dyn std::error::Error>> {
    let sqlite = SqliteStore::in_memory()?;
    let beta_path = PathBuf::from("/approved/beta.md");
    let beta_artifact = ArtifactId::new(1);
    let beta_hash = sillage_test_support::content_hash(10)?;
    EventLog::append(
        &sqlite,
        DomainEventEnvelope {
            id: EventId::new(1),
            event: DomainEvent::ParserStarted {
                artifact_id: beta_artifact,
                title: "beta".into(),
                source_path: beta_path.display().to_string(),
                content_hash: beta_hash.clone(),
                blob_id: BlobId::new(1),
            },
        },
    )?;
    let revision = sqlite.searchable_source_revision()?;
    let cached = sillage_domain::active_source_versions(&sqlite.scan_searchable_source_events()?);
    assert_eq!(
        cached.get(&beta_path).map(|(id, _, _)| *id),
        Some(beta_artifact)
    );

    EventLog::append(
        &sqlite,
        DomainEventEnvelope {
            id: EventId::new(2),
            event: DomainEvent::ParserStarted {
                artifact_id: ArtifactId::new(2),
                title: "other".into(),
                source_path: "/approved/other.md".into(),
                content_hash: sillage_test_support::content_hash(11)?,
                blob_id: BlobId::new(2),
            },
        },
    )?;
    let after_unrelated = active_sources_for_preview(&sqlite, Some((revision, &cached)))?;
    assert_eq!(
        after_unrelated.get(&beta_path).map(|(id, _, _)| *id),
        Some(beta_artifact)
    );

    EventLog::append(
        &sqlite,
        DomainEventEnvelope {
            id: EventId::new(3),
            event: DomainEvent::SourceBecameStale {
                artifact_id: beta_artifact,
                source_path: beta_path.display().to_string(),
                content_hash: beta_hash,
            },
        },
    )?;
    let after_revocation = active_sources_for_preview(&sqlite, Some((revision, &cached)))?;
    assert!(!after_revocation.contains_key(&beta_path));
    Ok(())
}
