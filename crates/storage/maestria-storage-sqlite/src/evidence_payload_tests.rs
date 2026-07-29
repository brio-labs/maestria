use super::*;
use maestria_domain::{
    BlobId, ContentHash, EvidenceKind, LogicalTick, SnapshotRef, WebEvidenceMetadata,
};

#[test]
fn web_snapshot_metadata_roundtrips_through_storage_payload()
-> Result<(), Box<dyn std::error::Error>> {
    let hash = ContentHash::new(format!("sha256:{}", "a".repeat(64)))?;
    let kind = EvidenceKind::WebSnapshot {
        url: "https://example.com/report".to_string(),
        snapshot: SnapshotRef::new(BlobId::new(7), hash),
        fetched_at: LogicalTick::new(11),
        metadata: WebEvidenceMetadata {
            published_at: Some("2026-07-16".to_string()),
            updated_at: Some("2026-07-17".to_string()),
            effective_at: None,
            accessed_at: Some("12".to_string()),
            content_type: Some("text/html".to_string()),
            primary_source: true,
            is_dynamic: true,
            is_paywalled: false,
        },
    };

    let stored = StoredEvidenceKind::from_domain(&kind);
    let decoded = serde_json::from_str::<StoredEvidenceKind>(&serde_json::to_string(&stored)?)?;

    assert_eq!(decoded.try_into_domain()?, kind);
    Ok(())
}

#[test]
fn omitted_file_snapshot_is_rejected_during_deserialization() {
    let error = serde_json::from_str::<StoredEvidenceKind>(
        r#"{"kind":"file_span","path":"notes.md","start":1,"end":1}"#,
    );
    assert!(error.is_err());
}

#[test]
fn null_file_snapshot_is_rejected_during_deserialization() {
    let error = serde_json::from_str::<StoredEvidenceKind>(
        r#"{"kind":"file_span","path":"notes.md","start":1,"end":1,"snapshot":null}"#,
    );
    assert!(error.is_err());
}

#[test]
fn invalid_file_range_fails_domain_decode() {
    let stored = StoredEvidenceKind::FileSpan {
        path: "notes.md".to_string(),
        start: 0,
        end: 1,
        snapshot: StoredSnapshotRef {
            blob_id: 1,
            content_hash: format!("sha256:{}", "a".repeat(64)),
        },
    };
    assert!(stored.try_into_domain().is_err());
}

#[test]
fn invalid_snapshot_hash_fails_domain_decode() {
    let stored = StoredEvidenceKind::FileSpan {
        path: "notes.md".to_string(),
        start: 1,
        end: 1,
        snapshot: StoredSnapshotRef {
            blob_id: 1,
            content_hash: "not-a-sha256".to_string(),
        },
    };
    assert!(stored.try_into_domain().is_err());
}
