use crate::{StructureNode, StructureNodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureTreeError {
    DuplicateNodeIds,
    InvalidRoot,
    DanglingLink,
    ParentCycle,
    SiblingCycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureTreeValidationFailure<E> {
    Invariant(StructureTreeError),
    Work(E),
}

/// Validate both link graphs without revisiting completed chains.
pub fn validate_structure_tree(
    root_id: StructureNodeId,
    nodes: &[StructureNode],
) -> Result<(), StructureTreeError> {
    let validation = validate_structure_tree_with_work(root_id, nodes, |_| {
        Ok::<(), std::convert::Infallible>(())
    });
    match validation {
        Ok(()) => Ok(()),
        Err(StructureTreeValidationFailure::Invariant(error)) => Err(error),
        Err(StructureTreeValidationFailure::Work(never)) => match never {},
    }
}

/// Validate the canonical tree invariants while reporting bounded work units.
///
/// The observer runs before each indexed operation, allowing callers to enforce
/// quotas and cooperative cancellation without changing validation semantics.
pub fn validate_structure_tree_with_work<E>(
    root_id: StructureNodeId,
    nodes: &[StructureNode],
    mut charge_work: impl FnMut(u64) -> Result<(), E>,
) -> Result<(), StructureTreeValidationFailure<E>> {
    let mut positions = Vec::with_capacity(nodes.len());
    for (position, node) in nodes.iter().enumerate() {
        charge_work(
            (std::mem::size_of::<StructureNodeId>()
                + std::mem::size_of::<(StructureNodeId, usize)>()) as u64,
        )
        .map_err(StructureTreeValidationFailure::Work)?;
        positions.push((node.id, position));
    }
    heap_sort_positions(&mut positions, &mut charge_work)
        .map_err(StructureTreeValidationFailure::Work)?;
    for pair in positions.windows(2) {
        charge_work((2 * std::mem::size_of::<StructureNodeId>()) as u64)
            .map_err(StructureTreeValidationFailure::Work)?;
        if pair[0].0 == pair[1].0 {
            return Err(StructureTreeValidationFailure::Invariant(
                StructureTreeError::DuplicateNodeIds,
            ));
        }
    }
    let mut root_count = 0;
    for node in nodes {
        charge_work(
            (std::mem::size_of::<Option<StructureNodeId>>()
                + std::mem::size_of::<StructureNodeId>()) as u64,
        )
        .map_err(StructureTreeValidationFailure::Work)?;
        if node.parent_id.is_none() {
            if node.id != root_id || root_count != 0 {
                return Err(StructureTreeValidationFailure::Invariant(
                    StructureTreeError::InvalidRoot,
                ));
            }
            root_count += 1;
        }
    }
    if root_count != 1 {
        return Err(StructureTreeValidationFailure::Invariant(
            StructureTreeError::InvalidRoot,
        ));
    }

    let mut visits = Vec::with_capacity(nodes.len());
    for _ in 0..nodes.len() {
        charge_work(std::mem::size_of::<usize>() as u64)
            .map_err(StructureTreeValidationFailure::Work)?;
        visits.push(0);
    }
    let parent_cycle = link_cycles(nodes, &positions, &mut visits, &mut charge_work, |node| {
        node.parent_id
    })?;
    for visit in &mut visits {
        charge_work(std::mem::size_of::<usize>() as u64)
            .map_err(StructureTreeValidationFailure::Work)?;
        *visit = 0;
    }
    let sibling_cycle = link_cycles(nodes, &positions, &mut visits, &mut charge_work, |node| {
        node.sibling_id
    })?;
    if parent_cycle {
        Err(StructureTreeValidationFailure::Invariant(
            StructureTreeError::ParentCycle,
        ))
    } else if sibling_cycle {
        Err(StructureTreeValidationFailure::Invariant(
            StructureTreeError::SiblingCycle,
        ))
    } else {
        Ok(())
    }
}

fn heap_sort_positions<E>(
    positions: &mut [(StructureNodeId, usize)],
    charge_work: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<(), E> {
    let len = positions.len();
    for root in (0..len / 2).rev() {
        sift_down(positions, root, len, charge_work)?;
    }
    for end in (1..len).rev() {
        charge_work((2 * std::mem::size_of::<(StructureNodeId, usize)>()) as u64)?;
        positions.swap(0, end);
        sift_down(positions, 0, end, charge_work)?;
    }
    Ok(())
}

fn sift_down<E>(
    positions: &mut [(StructureNodeId, usize)],
    mut root: usize,
    end: usize,
    charge_work: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<(), E> {
    while root < end / 2 {
        let left = root * 2 + 1;
        let right = left + 1;
        charge_work((2 * std::mem::size_of::<StructureNodeId>()) as u64)?;
        let mut largest = if positions[root].0 < positions[left].0 {
            left
        } else {
            root
        };
        if right < end {
            charge_work((2 * std::mem::size_of::<StructureNodeId>()) as u64)?;
            if positions[largest].0 < positions[right].0 {
                largest = right;
            }
        }
        if largest == root {
            return Ok(());
        }
        charge_work((2 * std::mem::size_of::<(StructureNodeId, usize)>()) as u64)?;
        positions.swap(root, largest);
        root = largest;
    }
    Ok(())
}

fn link_cycles<E>(
    nodes: &[StructureNode],
    positions: &[(StructureNodeId, usize)],
    visits: &mut [usize],
    charge_work: &mut impl FnMut(u64) -> Result<(), E>,
    next: fn(&StructureNode) -> Option<StructureNodeId>,
) -> Result<bool, StructureTreeValidationFailure<E>> {
    let mut cycle = false;
    for start in 0..nodes.len() {
        charge_work(std::mem::size_of::<usize>() as u64)
            .map_err(StructureTreeValidationFailure::Work)?;
        if visits[start] != 0 {
            continue;
        }
        let traversal = start + 1;
        let mut current = start;
        loop {
            charge_work(std::mem::size_of::<usize>() as u64)
                .map_err(StructureTreeValidationFailure::Work)?;
            let visit = visits[current];
            if visit != 0 {
                cycle |= visit == traversal;
                break;
            }
            charge_work(std::mem::size_of::<usize>() as u64)
                .map_err(StructureTreeValidationFailure::Work)?;
            visits[current] = traversal;
            charge_work(std::mem::size_of::<Option<StructureNodeId>>() as u64)
                .map_err(StructureTreeValidationFailure::Work)?;
            let Some(next_id) = next(&nodes[current]) else {
                break;
            };
            let position = find_position(positions, next_id, charge_work)
                .map_err(StructureTreeValidationFailure::Work)?
                .ok_or(StructureTreeValidationFailure::Invariant(
                    StructureTreeError::DanglingLink,
                ))?;
            charge_work(std::mem::size_of::<usize>() as u64)
                .map_err(StructureTreeValidationFailure::Work)?;
            current = positions[position].1;
        }
    }
    // Examine every component even after a cycle: dangling links retain
    // precedence over either cycle error, including in the other link graph.
    Ok(cycle)
}

fn find_position<E>(
    positions: &[(StructureNodeId, usize)],
    id: StructureNodeId,
    charge_work: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<Option<usize>, E> {
    let mut low = 0;
    let mut high = positions.len();
    while low < high {
        charge_work(std::mem::size_of::<StructureNodeId>() as u64)?;
        let middle = low + (high - low) / 2;
        if positions[middle].0 < id {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low < positions.len() {
        charge_work(std::mem::size_of::<StructureNodeId>() as u64)?;
        if positions[low].0 == id {
            return Ok(Some(low));
        }
    }
    Ok(None)
}
