use super::super::contract_tests::*;
use super::super::graph_contract_tests::assert_graph_index_contract;
use super::super::*;

#[test]
fn in_memory_full_text_index_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_full_text_index_round_trip(&InMemoryFullTextIndex::new())?;
    Ok(())
}

#[test]
fn in_memory_vector_index_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_vector_index_contract(&InMemoryVectorIndex::new())?;
    Ok(())
}

#[test]
fn in_memory_graph_index_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_graph_index_contract(&InMemoryGraphIndex::new())?;
    Ok(())
}
