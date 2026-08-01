use super::super::contract_tests::*;
use super::super::*;
use maestria_domain::{
    ArtifactId, Evidence, EvidenceId, EvidenceKind, LogicalTick, ValidationReportId,
};

#[test]
fn in_memory_artifact_repository_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_artifact_repository_round_trip(&InMemoryArtifactRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_chunk_repository_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_chunk_repository_round_trip(&InMemoryChunkRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_web_fetcher_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = InMemoryWebFetcher::new();
    fetcher.seed("https://example.com/test", "<html><body>test</body></html>")?;
    assert_web_fetcher_contract(
        &fetcher,
        "https://example.com/test",
        "<html><body>test</body></html>",
    )?;

    let missing_res = fetcher.fetch("https://example.com/not-found-anywhere", usize::MAX);
    assert!(
        matches!(missing_res, Err(PortError::NotFound)),
        "Missing URLs must map to PortError::NotFound, got {:?}",
        missing_res
    );

    let zero_limit = fetcher.fetch("https://example.com/test", 0);
    assert!(zero_limit.is_err_and(|error| error.is_invalid_input()));
    let too_large = fetcher.fetch("https://example.com/test", 1);
    assert!(too_large.is_err_and(|error| error.is_invalid_input()));

    Ok(())
}

#[test]
fn in_memory_card_repository_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_card_repository_round_trip(&InMemoryCardRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_evidence_repository_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_evidence_repository_round_trip(&InMemoryEvidenceRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_evidence_put_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let repo = InMemoryEvidenceRepository::new();
    let evidence = Evidence {
        id: EvidenceId::new(100),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "test excerpt".to_string(),
        observed_at: LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    };
    // First insert succeeds
    repo.put(evidence.clone())?;
    // Identical retry is idempotent
    repo.put(evidence.clone())?;
    // Stored value is unchanged
    let stored = repo
        .get(evidence.id)?
        .ok_or_else(|| std::io::Error::other("stored evidence missing"))?;
    assert_eq!(stored, evidence);
    Ok(())
}

#[test]
fn in_memory_evidence_repository_satisfies_replace_contract()
-> Result<(), Box<dyn std::error::Error>> {
    assert_evidence_repository_replace_contract(&InMemoryEvidenceRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_evidence_replace_overwrites_existing() -> Result<(), Box<dyn std::error::Error>> {
    let repo = InMemoryEvidenceRepository::new();
    let original = Evidence {
        id: EvidenceId::new(300),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "malformed".to_string(),
        observed_at: LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    };
    repo.put(original.clone())?;

    let replacement = Evidence {
        id: EvidenceId::new(300),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2),
        },
        excerpt: "corrected".to_string(),
        observed_at: LogicalTick::new(2),
        security: maestria_domain::SecurityMetadata::default(),
    };

    // put rejects different content
    let Err(err) = repo.put(replacement.clone()) else {
        return Err("expected error".into());
    };
    assert!(matches!(err, PortError::Conflict { .. }));

    // replace succeeds with different content
    repo.replace(replacement.clone())?;

    let stored = repo
        .get(EvidenceId::new(300))?
        .ok_or_else(|| std::io::Error::other("replacement evidence missing"))?;
    assert_eq!(stored, replacement);
    Ok(())
}

#[test]
fn in_memory_evidence_put_rejects_conflicting_overwrite() -> Result<(), Box<dyn std::error::Error>>
{
    let repo = InMemoryEvidenceRepository::new();
    let first = Evidence {
        id: EvidenceId::new(200),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "original".to_string(),
        observed_at: LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    };
    repo.put(first.clone())?;

    let conflict = Evidence {
        id: EvidenceId::new(200), // same id
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2), // different report_id
        },
        excerpt: "different".to_string(),
        observed_at: LogicalTick::new(2),
        security: maestria_domain::SecurityMetadata::default(),
    };
    let Err(err) = repo.put(conflict) else {
        return Err("expected error".into());
    };
    assert!(
        matches!(err, PortError::Conflict { .. }),
        "conflicting put must return Conflict, got {err:?}"
    );

    // Original is preserved
    let stored = repo
        .get(first.id)?
        .ok_or_else(|| std::io::Error::other("original evidence missing"))?;
    assert_eq!(stored, first);
    Ok(())
}

#[test]
fn in_memory_blob_store_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_blob_store_round_trip(&InMemoryBlobStore::new())?;
    Ok(())
}
