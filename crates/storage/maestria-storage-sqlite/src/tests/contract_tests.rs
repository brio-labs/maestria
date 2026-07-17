use crate::SqliteStore;
use maestria_ports::contract_tests;

#[test]
fn satisfies_shared_artifact_repository_contract() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;

    contract_tests::assert_artifact_repository_round_trip(&store)?;
    Ok(())
}

#[test]
fn satisfies_shared_event_log_contract() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;

    contract_tests::assert_event_log_round_trip(&store)?;
    Ok(())
}

#[test]
fn satisfies_shared_chunk_repository_contract() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;

    contract_tests::assert_chunk_repository_round_trip(&store)?;
    Ok(())
}

#[test]
fn satisfies_shared_card_repository_contract() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;

    contract_tests::assert_card_repository_round_trip(&store)?;
    Ok(())
}

#[test]
fn satisfies_shared_evidence_repository_contract() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;

    contract_tests::assert_evidence_repository_round_trip(&store)?;
    Ok(())
}

#[test]
fn satisfies_shared_evidence_replace_contract() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    contract_tests::assert_evidence_repository_replace_contract(&store)?;
    Ok(())
}
