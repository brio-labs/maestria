use super::super::*;
use maestria_domain::{ContentRange, StructureNode, StructureNodeId, StructureNodeType};

fn structure_node(id: u64, parent_id: Option<u64>, sibling_id: Option<u64>) -> StructureNode {
    StructureNode {
        id: StructureNodeId::new(id),
        parent_id: parent_id.map(StructureNodeId::new),
        sibling_id: sibling_id.map(StructureNodeId::new),
        node_type: StructureNodeType::Document,
        source_range: ContentRange { start: 0, end: 0 },
        page: None,
        section_path: Vec::new(),
        parser_generation: "test".to_string(),
        schema_generation: "test".to_string(),
        language: None,
    }
}

#[test]
fn document_tree_rejects_invalid_topologies() -> Result<(), PortError> {
    let root_id = StructureNodeId::new(1);
    let root = structure_node(1, None, None);

    assert!(
        DocumentTree::new(root_id, vec![root.clone(), root])
            .is_err_and(|error| { error.is_invalid_input() })
    );
    assert!(
        DocumentTree::new(root_id, vec![structure_node(2, None, None)])
            .is_err_and(|error| error.is_invalid_input())
    );
    assert!(
        DocumentTree::new(
            root_id,
            vec![
                structure_node(1, None, None),
                structure_node(2, Some(99), None)
            ],
        )
        .is_err_and(|error| error.is_invalid_input())
    );
    assert!(
        DocumentTree::new(
            root_id,
            vec![
                structure_node(1, None, None),
                structure_node(2, Some(3), None),
                structure_node(3, Some(2), None),
            ],
        )
        .is_err_and(|error| error.is_invalid_input())
    );
    assert!(
        DocumentTree::new(
            root_id,
            vec![
                structure_node(1, None, Some(2)),
                structure_node(2, Some(1), Some(1)),
            ],
        )
        .is_err_and(|error| error.is_invalid_input())
    );
    Ok(())
}
