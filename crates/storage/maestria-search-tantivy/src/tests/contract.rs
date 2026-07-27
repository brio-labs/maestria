use crate::TantivyFullTextIndex;
use maestria_ports::contract_tests::assert_full_text_index_round_trip;

#[test]
fn satisfies_shared_full_text_index_contract() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;
    assert_full_text_index_round_trip(&index)?;
    Ok(())
}
