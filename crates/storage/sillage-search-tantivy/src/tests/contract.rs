use crate::test_ram_index;
use sillage_ports::contract_tests::assert_full_text_index_round_trip;

#[test]
fn satisfies_shared_full_text_index_contract() -> Result<(), Box<dyn std::error::Error>> {
    let index = test_ram_index()?;
    assert_full_text_index_round_trip(&index)?;
    Ok(())
}
