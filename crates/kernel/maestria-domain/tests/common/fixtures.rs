use maestria_domain::*;

/// A canonical content hash for deterministic test fixtures.
/// Format: "sha256:" followed by 64 hex characters.
pub fn test_content_hash() -> Result<ContentHash, Box<dyn std::error::Error>> {
    Ok(ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?)
}

/// A root [`StructureNode`] with the given id and minimal defaults.
pub fn tree_root_node(id: StructureNodeId) -> StructureNode {
    StructureNode {
        id,
        parent_id: None,
        sibling_id: None,
        node_type: StructureNodeType::Document,
        source_range: ContentRange { start: 0, end: 0 },
        page: None,
        section_path: vec![],
        parser_generation: "test".to_string(),
        schema_generation: "test".to_string(),
        language: None,
    }
}
