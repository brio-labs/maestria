use super::test_helpers::adapter;
use maestria_ports::contract_tests::assert_harness_adapter_round_trip;

// ── shared contract suite (Rule 25) ────────────────────────────────

#[tokio::test]
async fn harness_adapter_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_harness_adapter_round_trip(&adapter()).await?;
    Ok(())
}
