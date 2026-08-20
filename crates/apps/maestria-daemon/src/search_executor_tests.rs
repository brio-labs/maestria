use super::*;
use maestria_code_intel::REPOSITORY_CODE_INDEX_FILENAME;
use maestria_domain::{ArtifactId, ArtifactVersionId, BlobId, DomainEvent, EventId};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_INDEX_TEST_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_layout() -> InstanceLayout {
    let id = NEXT_INDEX_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "maestria-daemon-runtime-code-index-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    let _ = fs::create_dir_all(&path);
    InstanceLayout::for_root(path)
}

#[test]
fn load_repository_code_index_returns_none_when_missing() -> Result<(), Box<dyn std::error::Error>>
{
    let layout = temporary_layout();
    fs::create_dir_all(&layout.system_dir)?;
    let index = load_repository_code_index_with_exclusions(&layout, None)?;
    assert!(index.is_none());
    Ok(())
}

#[test]
fn load_repository_code_index_rejects_malformed_file_as_typed_error()
-> Result<(), Box<dyn std::error::Error>> {
    let layout = temporary_layout();
    fs::create_dir_all(&layout.system_dir)?;
    let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
    fs::write(&index_path, "not valid json")?;
    let result = load_repository_code_index_with_exclusions(&layout, None);
    assert!(result.is_err());
    assert!(matches!(
        result.err(),
        Some(maestria_code_intel::CodeIntelError::Persist { .. })
    ));
    Ok(())
}

#[test]
fn parser_started_then_source_became_stale_excludes_version()
-> Result<(), Box<dyn std::error::Error>> {
    let path = "src/main.rs".to_string();
    let artifact_id = ArtifactId::new(1);
    let events = vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            event: DomainEvent::ParserStarted {
                artifact_id,
                title: "main".to_string(),
                source_path: path.clone(),
                content_hash: maestria_test_support::content_hash(10)?,
                blob_id: BlobId::new(1),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            event: DomainEvent::SourceBecameStale {
                artifact_id,
                source_path: path.clone(),
                content_hash: maestria_test_support::content_hash(10)?,
            },
        },
    ];
    let sources = maestria_domain::active_source_versions(&events);
    let active = reconcile_active_versions(&sources);
    assert!(active.is_empty());
    Ok(())
}

#[test]
fn re_ingestion_after_stale_reactivates_version() -> Result<(), Box<dyn std::error::Error>> {
    let path = "src/main.rs".to_string();
    let artifact_id_v1 = ArtifactId::new(1);
    let artifact_id_v2 = ArtifactId::new(2);
    let events = vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            event: DomainEvent::ParserStarted {
                artifact_id: artifact_id_v1,
                title: "main".to_string(),
                source_path: path.clone(),
                content_hash: maestria_test_support::content_hash(10)?,
                blob_id: BlobId::new(1),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            event: DomainEvent::SourceBecameStale {
                artifact_id: artifact_id_v1,
                source_path: path.clone(),
                content_hash: maestria_test_support::content_hash(10)?,
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            event: DomainEvent::ParserStarted {
                artifact_id: artifact_id_v2,
                title: "main".to_string(),
                source_path: path.clone(),
                content_hash: maestria_test_support::content_hash(13)?,
                blob_id: BlobId::new(2),
            },
        },
    ];
    let sources = maestria_domain::active_source_versions(&events);
    let active = reconcile_active_versions(&sources);
    assert_eq!(active.len(), 1);
    assert!(active.contains(&ArtifactVersionId::new(artifact_id_v2.value())));
    Ok(())
}

#[test]
fn latest_by_path_semantics_preserved_across_mixed_events() -> Result<(), Box<dyn std::error::Error>>
{
    let path_a = "src/a.rs".to_string();
    let path_b = "src/b.rs".to_string();
    let id_a1 = ArtifactId::new(1);
    let id_a2 = ArtifactId::new(2);
    let id_b1 = ArtifactId::new(3);
    let events = vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            event: DomainEvent::ParserStarted {
                artifact_id: id_a1,
                title: "a".to_string(),
                source_path: path_a.clone(),
                content_hash: maestria_test_support::content_hash(1)?,
                blob_id: BlobId::new(1),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            event: DomainEvent::ParserStarted {
                artifact_id: id_b1,
                title: "b".to_string(),
                source_path: path_b.clone(),
                content_hash: maestria_test_support::content_hash(2)?,
                blob_id: BlobId::new(2),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            event: DomainEvent::ParserStarted {
                artifact_id: id_a2,
                title: "a".to_string(),
                source_path: path_a.clone(),
                content_hash: maestria_test_support::content_hash(3)?,
                blob_id: BlobId::new(3),
            },
        },
    ];
    let sources = maestria_domain::active_source_versions(&events);
    let active = reconcile_active_versions(&sources);
    assert_eq!(active.len(), 2);
    assert!(active.contains(&ArtifactVersionId::new(id_a2.value())));
    assert!(active.contains(&ArtifactVersionId::new(id_b1.value())));
    Ok(())
}

#[test]
fn document_tree_captured_version_overrides_placeholder() -> Result<(), Box<dyn std::error::Error>>
{
    let path = "src/main.rs".to_string();
    let artifact_id = ArtifactId::new(1);
    let content_hash = maestria_test_support::content_hash(10)?;
    let real_version = content_hash.version_id()?;
    assert_ne!(
        real_version,
        ArtifactVersionId::new(artifact_id.value()),
        "content-derived version must not equal the artifact-id placeholder"
    );
    let events = vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            event: DomainEvent::ParserStarted {
                artifact_id,
                title: "main".to_string(),
                source_path: path.clone(),
                content_hash: content_hash.clone(),
                blob_id: BlobId::new(1),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            event: DomainEvent::DocumentTreeCaptured {
                artifact_id,
                artifact_version_id: real_version,
                content_hash,
                root_id: maestria_domain::StructureNodeId::new(1),
                nodes: Vec::new(),
            },
        },
    ];
    let sources = maestria_domain::active_source_versions(&events);
    let active = reconcile_active_versions(&sources);
    assert_eq!(active.len(), 1);
    assert!(
        active.contains(&real_version),
        "active versions must use the content-addressed tree-captured version"
    );
    assert!(!active.contains(&ArtifactVersionId::new(artifact_id.value())));
    Ok(())
}
