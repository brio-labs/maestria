use super::super::contract_tests::*;
use super::super::*;

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
fn in_memory_realm_read_grant_repository_satisfies_contract()
-> Result<(), Box<dyn std::error::Error>> {
    assert_realm_read_grant_repository_contract(&InMemoryRealmReadGrantRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_blob_store_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_blob_store_round_trip(&InMemoryBlobStore::new())?;
    Ok(())
}
