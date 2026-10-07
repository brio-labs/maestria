use crate::{StructureNode, StructureNodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureTreeError {
    DuplicateNodeIds,
    InvalidRoot,
    DanglingLink,
    ParentCycle,
    SiblingCycle,
}

/// Validate both link graphs without revisiting completed chains.
pub fn validate_structure_tree(
    root_id: StructureNodeId,
    nodes: &[StructureNode],
) -> Result<(), StructureTreeError> {
    let mut positions: Vec<_> = nodes
        .iter()
        .enumerate()
        .map(|(position, node)| (node.id, position))
        .collect();
    positions.sort_unstable_by_key(|(id, _)| *id);
    if positions.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(StructureTreeError::DuplicateNodeIds);
    }
    let mut roots = nodes.iter().filter(|node| node.parent_id.is_none());
    if roots.next().is_none_or(|root| root.id != root_id) || roots.next().is_some() {
        return Err(StructureTreeError::InvalidRoot);
    }

    let mut visits = vec![0; nodes.len()];
    let parent_cycle = link_cycles(nodes, &positions, &mut visits, |node| node.parent_id)?;
    visits.fill(0);
    let sibling_cycle = link_cycles(nodes, &positions, &mut visits, |node| node.sibling_id)?;
    if parent_cycle {
        Err(StructureTreeError::ParentCycle)
    } else if sibling_cycle {
        Err(StructureTreeError::SiblingCycle)
    } else {
        Ok(())
    }
}

fn link_cycles(
    nodes: &[StructureNode],
    positions: &[(StructureNodeId, usize)],
    visits: &mut [usize],
    next: fn(&StructureNode) -> Option<StructureNodeId>,
) -> Result<bool, StructureTreeError> {
    let mut cycle = false;
    for start in 0..nodes.len() {
        if visits[start] != 0 {
            continue;
        }
        let traversal = start + 1;
        let mut current = start;
        loop {
            if visits[current] != 0 {
                cycle |= visits[current] == traversal;
                break;
            }
            visits[current] = traversal;
            let Some(next_id) = next(&nodes[current]) else {
                break;
            };
            let position = positions
                .binary_search_by_key(&next_id, |(id, _)| *id)
                .map_err(|_| StructureTreeError::DanglingLink)?;
            current = positions[position].1;
        }
    }
    // Examine every component even after a cycle: dangling links retain
    // precedence over either cycle error, including in the other link graph.
    Ok(cycle)
}
