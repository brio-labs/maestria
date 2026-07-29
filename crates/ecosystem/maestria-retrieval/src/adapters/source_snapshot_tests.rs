use super::*;
use maestria_ports::{BlobStore, InMemoryBlobStore, PortError};
use std::sync::atomic::{AtomicUsize, Ordering};
type SnapshotFixture = (SourceSnapshotVerifier, Evidence, maestria_domain::Artifact);
type CountingSnapshotFixture = (
    SourceSnapshotVerifier,
    Evidence,
    maestria_domain::Artifact,
    Arc<CountingBlobStore>,
);

struct CountingBlobStore {
    bytes: Vec<u8>,
    gets: AtomicUsize,
}

impl BlobStore for CountingBlobStore {
    fn put(&self, _bytes: Vec<u8>) -> Result<maestria_domain::BlobId, PortError> {
        Ok(maestria_domain::BlobId::new(1))
    }

    fn get(&self, _id: maestria_domain::BlobId) -> Result<Vec<u8>, PortError> {
        self.gets.fetch_add(1, Ordering::Relaxed);
        Ok(self.bytes.clone())
    }
}

fn evidence_for_source(
    source: &[u8],
    range: maestria_domain::LineRange,
    excerpt: &str,
) -> Result<SnapshotFixture, Box<dyn std::error::Error>> {
    let blobs = Arc::new(InMemoryBlobStore::new());
    let snapshot = blobs.put(source.to_vec())?;
    let content_hash = maestria_domain::ContentHash::new(maestria_domain::content_hash(source))?;
    let evidence = Evidence {
        id: maestria_domain::EvidenceId::new(1),
        artifact_id: maestria_domain::ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "notes.md".to_string(),
            range,
            snapshot: maestria_domain::SnapshotRef::new(snapshot, content_hash),
        },
        excerpt: excerpt.to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: Default::default(),
    };
    let artifact = maestria_domain::Artifact {
        id: maestria_domain::ArtifactId::new(1),
        title: "notes.md".to_string(),
        chunk_ids: Default::default(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: maestria_domain::IndexStatus::Indexed,
        content_hash: Some(maestria_domain::content_hash(source)),
        parse_status: None,
        security: Default::default(),
    };
    Ok((SourceSnapshotVerifier::new(blobs), evidence, artifact))
}

fn file_evidence(
    range: maestria_domain::LineRange,
    excerpt: &str,
) -> Result<SnapshotFixture, Box<dyn std::error::Error>> {
    evidence_for_source(b"alpha line\nbeta line\n", range, excerpt)
}

fn web_evidence_for_source(
    source: &[u8],
    excerpt: &str,
) -> Result<CountingSnapshotFixture, Box<dyn std::error::Error>> {
    let blobs = Arc::new(CountingBlobStore {
        bytes: source.to_vec(),
        gets: AtomicUsize::new(0),
    });
    let content_hash = maestria_domain::ContentHash::new(maestria_domain::content_hash(source))?;
    let evidence = Evidence {
        id: maestria_domain::EvidenceId::new(1),
        artifact_id: maestria_domain::ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::WebSnapshot {
            url: "https://example.test/source".to_string(),
            snapshot: maestria_domain::SnapshotRef::new(
                maestria_domain::BlobId::new(1),
                content_hash,
            ),
            fetched_at: maestria_domain::LogicalTick::new(1),
            metadata: Default::default(),
        },
        excerpt: excerpt.to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: Default::default(),
    };
    let artifact = maestria_domain::Artifact {
        id: maestria_domain::ArtifactId::new(1),
        title: "web source".to_string(),
        chunk_ids: Default::default(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: maestria_domain::IndexStatus::Indexed,
        content_hash: Some(maestria_domain::content_hash(source)),
        parse_status: None,
        security: Default::default(),
    };
    Ok((
        SourceSnapshotVerifier::new(blobs.clone()),
        evidence,
        artifact,
        blobs,
    ))
}

fn pdf_evidence_for_source(
    source: &[u8],
) -> Result<CountingSnapshotFixture, Box<dyn std::error::Error>> {
    pdf_evidence_with_stored_bytes(source, source)
}

fn pdf_evidence_with_stored_bytes(
    expected_source: &[u8],
    stored_bytes: &[u8],
) -> Result<CountingSnapshotFixture, Box<dyn std::error::Error>> {
    let blobs = Arc::new(CountingBlobStore {
        bytes: stored_bytes.to_vec(),
        gets: AtomicUsize::new(0),
    });
    let hash = maestria_domain::ContentHash::new(maestria_domain::content_hash(expected_source))?;
    let evidence = Evidence {
        id: maestria_domain::EvidenceId::new(2),
        artifact_id: maestria_domain::ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::PdfSpan {
            snapshot: maestria_domain::SnapshotRef::new(maestria_domain::BlobId::new(1), hash),
            page_start: 1,
            page_end: 1,
        },
        excerpt: "figure".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: Default::default(),
    };
    let artifact = maestria_domain::Artifact {
        id: maestria_domain::ArtifactId::new(1),
        title: "pdf source".to_string(),
        chunk_ids: Default::default(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: maestria_domain::IndexStatus::Indexed,
        content_hash: Some(maestria_domain::content_hash(expected_source)),
        parse_status: None,
        security: Default::default(),
    };
    Ok((
        SourceSnapshotVerifier::new(blobs.clone()),
        evidence,
        artifact,
        blobs,
    ))
}

#[test]
fn pdf_snapshot_rejects_cross_artifact_identity_before_blob_read()
-> Result<(), Box<dyn std::error::Error>> {
    let (verifier, mut evidence, artifact, blobs) = pdf_evidence_for_source(b"pdf bytes")?;
    evidence.artifact_id = maestria_domain::ArtifactId::new(2);

    let result = verifier.verify(&evidence, &artifact);
    assert!(matches!(
        result,
        Err(RetrievalError::Internal(message)) if message.contains("belongs to artifact")
    ));
    assert_eq!(blobs.gets.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn pdf_snapshot_rejects_hash_mismatch_before_blob_read() -> Result<(), Box<dyn std::error::Error>> {
    let (verifier, evidence, mut artifact, blobs) = pdf_evidence_for_source(b"pdf bytes")?;
    artifact.content_hash = Some(maestria_domain::content_hash(b"other bytes"));

    let result = verifier.verify(&evidence, &artifact);
    assert!(matches!(
        result,
        Err(RetrievalError::Internal(message))
            if message.contains("source snapshot hash does not match owning artifact")
    ));
    assert_eq!(blobs.gets.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn pdf_snapshot_rejects_retrieved_byte_hash_mismatch_after_blob_read()
-> Result<(), Box<dyn std::error::Error>> {
    let (verifier, evidence, artifact, blobs) =
        pdf_evidence_with_stored_bytes(b"expected bytes", b"tampered bytes")?;

    let result = verifier.verify(&evidence, &artifact);
    assert!(matches!(
        result,
        Err(RetrievalError::Internal(message))
            if message.contains("source snapshot verification failed")
    ));
    assert_eq!(blobs.gets.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn pdf_snapshot_rejects_empty_retrieved_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let (verifier, evidence, artifact, blobs) =
        pdf_evidence_with_stored_bytes(b"expected bytes", b"")?;

    let result = verifier.verify(&evidence, &artifact);
    assert!(matches!(
        result,
        Err(RetrievalError::Internal(message))
            if message.contains("source snapshot verification failed")
    ));
    assert_eq!(blobs.gets.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn web_snapshot_rejects_cross_artifact_identity_before_blob_read()
-> Result<(), Box<dyn std::error::Error>> {
    let (verifier, mut evidence, artifact, blobs) =
        web_evidence_for_source(b"h1 web evidence\n", "h1 web evidence")?;
    evidence.artifact_id = maestria_domain::ArtifactId::new(2);

    let result = verifier.verify(&evidence, &artifact);
    assert!(matches!(
        result,
        Err(RetrievalError::Internal(message)) if message.contains("belongs to artifact")
    ));
    assert_eq!(blobs.gets.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn web_snapshot_rejects_hash_mismatch_before_blob_read() -> Result<(), Box<dyn std::error::Error>> {
    let (verifier, evidence, mut artifact, blobs) =
        web_evidence_for_source(b"h2 web evidence\n", "h2 web evidence")?;
    artifact.content_hash = Some(maestria_domain::content_hash(b"h1 artifact\n"));

    let result = verifier.verify(&evidence, &artifact);
    assert!(matches!(
        result,
        Err(RetrievalError::Internal(message))
            if message.contains("source snapshot hash does not match owning artifact")
    ));
    assert_eq!(blobs.gets.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn web_snapshot_valid_owner_and_hash_reaches_blob_verification()
-> Result<(), Box<dyn std::error::Error>> {
    let (verifier, evidence, artifact, blobs) =
        web_evidence_for_source(b"valid web evidence\n", "valid web evidence")?;
    verifier.verify(&evidence, &artifact)?;
    assert_eq!(blobs.gets.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn file_snapshot_range_must_bound_its_excerpt() -> Result<(), Box<dyn std::error::Error>> {
    let (verifier, valid, artifact) =
        file_evidence(maestria_domain::LineRange::new(1, 1)?, "alpha line")?;
    verifier.verify(&valid, &artifact)?;

    let (verifier, out_of_bounds, artifact) =
        file_evidence(maestria_domain::LineRange::new(1, 3)?, "alpha line")?;
    assert!(verifier.verify(&out_of_bounds, &artifact).is_err());

    let (verifier, wrong_range, artifact) =
        file_evidence(maestria_domain::LineRange::new(2, 2)?, "alpha line")?;
    assert!(verifier.verify(&wrong_range, &artifact).is_err());
    Ok(())
}

#[test]
fn strict_snapshot_verifier_rejects_invalid_utf8() -> Result<(), Box<dyn std::error::Error>> {
    let (verifier, evidence, artifact) = evidence_for_source(
        b"valid \xff text",
        maestria_domain::LineRange::new(1, 1)?,
        "valid",
    )?;
    assert!(verifier.verify(&evidence, &artifact).is_err());
    Ok(())
}

#[test]
fn strict_snapshot_verifier_matches_only_the_exact_selected_range()
-> Result<(), Box<dyn std::error::Error>> {
    let (verifier, exact, artifact) = evidence_for_source(
        b"outside\ninside\noutside",
        maestria_domain::LineRange::new(2, 2)?,
        "inside",
    )?;
    verifier.verify(&exact, &artifact)?;

    let (verifier, outside, artifact) = evidence_for_source(
        b"outside\ninside\noutside",
        maestria_domain::LineRange::new(2, 2)?,
        "outside",
    )?;
    assert!(verifier.verify(&outside, &artifact).is_err());
    Ok(())
}

#[test]
fn strict_snapshot_verifier_handles_crlf_and_final_line() -> Result<(), Box<dyn std::error::Error>>
{
    let (verifier, evidence, artifact) = evidence_for_source(
        b"first\r\nlast",
        maestria_domain::LineRange::new(2, 2)?,
        "last",
    )?;
    verifier.verify(&evidence, &artifact)?;
    Ok(())
}

#[test]
fn strict_snapshot_verifier_handles_multibyte_utf8() -> Result<(), Box<dyn std::error::Error>> {
    let (verifier, evidence, artifact) = evidence_for_source(
        "alpha\n猫".as_bytes(),
        maestria_domain::LineRange::new(2, 2)?,
        "猫",
    )?;
    verifier.verify(&evidence, &artifact)?;
    Ok(())
}

#[test]
fn file_snapshot_hash_must_match_owning_artifact() -> Result<(), Box<dyn std::error::Error>> {
    let (verifier, evidence, mut artifact) = evidence_for_source(
        b"h2 evidence and blob\n",
        maestria_domain::LineRange::new(1, 1)?,
        "h2 evidence and blob",
    )?;
    artifact.content_hash = Some(maestria_domain::content_hash(b"h1 artifact\n"));
    let mismatch = verifier.verify(&evidence, &artifact);
    assert!(matches!(
        mismatch,
        Err(RetrievalError::Internal(message))
            if message.contains("source snapshot hash does not match owning artifact")
    ));

    artifact.content_hash = None;
    let missing = verifier.verify(&evidence, &artifact);
    assert!(matches!(
        missing,
        Err(RetrievalError::Internal(message))
            if message.contains("source snapshot hash does not match owning artifact")
    ));

    let (verifier, evidence, artifact) = evidence_for_source(
        b"h2 evidence and blob\n",
        maestria_domain::LineRange::new(1, 1)?,
        "h2 evidence and blob",
    )?;
    verifier.verify(&evidence, &artifact)?;
    Ok(())
}
