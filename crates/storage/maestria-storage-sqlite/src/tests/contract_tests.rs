use crate::SqliteStore;
use maestria_ports::contract_tests;

#[test]
fn satisfies_shared_artifact_repository_contract() {
    let store = SqliteStore::in_memory().expect("test setup");

    contract_tests::assert_artifact_repository_round_trip(&store);
}

#[test]
fn satisfies_shared_event_log_contract() {
    let store = SqliteStore::in_memory().expect("test setup");

    contract_tests::assert_event_log_round_trip(&store);
}

#[test]
fn satisfies_shared_chunk_repository_contract() {
    let store = SqliteStore::in_memory().expect("test setup");

    contract_tests::assert_chunk_repository_round_trip(&store);
}

#[test]
fn satisfies_shared_card_repository_contract() {
    let store = SqliteStore::in_memory().expect("test setup");

    contract_tests::assert_card_repository_round_trip(&store);
}

#[test]
fn satisfies_shared_evidence_repository_contract() {
    let store = SqliteStore::in_memory().expect("test setup");

    contract_tests::assert_evidence_repository_round_trip(&store);
}

#[test]
fn satisfies_shared_evidence_replace_contract() {
    let store = SqliteStore::in_memory().expect("test setup");
    contract_tests::assert_evidence_repository_replace_contract(&store);
}
