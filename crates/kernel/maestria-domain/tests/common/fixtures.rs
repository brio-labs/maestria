use maestria_domain::*;
#[path = "content_hash.rs"]
mod content_hash;
pub use content_hash::test_content_hash;

/// A root [`StructureNode`] with the given id and minimal defaults.
pub fn tree_root_node(id: StructureNodeId) -> Result<StructureNode, Box<dyn std::error::Error>> {
    Ok(StructureNode {
        id,
        parent_id: None,
        sibling_id: None,
        node_type: StructureNodeType::Document,
        source_range: ContentRange::new(0, 0)?,
        page: None,
        section_path: vec![],
        parser_generation: "test".to_string(),
        schema_generation: "test".to_string(),
        language: None,
    })
}
