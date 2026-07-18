use super::context::{MAX_CONTEXT_DEPTH, MAX_CONTEXT_NODES, MAX_CONTEXT_RELATION_VISITS};
use crate::context::{ContextDirection, RepositoryContextQuery};
use crate::{CodeQuery, CodeRelationKind, CodeRelationRecord, QueryResult, SymbolRecord};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub(crate) struct DiscoveredNode {
    pub(crate) depth: usize,
    pub(crate) seed_record_ids: BTreeSet<String>,
}

#[derive(Debug)]
pub(crate) struct RepositoryContextEdgeState {
    pub(crate) relation: CodeRelationRecord,
    pub(crate) depth: usize,
    pub(crate) seed_record_ids: BTreeSet<String>,
}

pub(crate) struct NormalizedContextQuery {
    pub(crate) query: CodeQuery,
    pub(crate) direction: ContextDirection,
    pub(crate) relation_kinds: Option<Vec<CodeRelationKind>>,
    pub(crate) max_depth: usize,
    pub(crate) max_nodes: usize,
}

#[derive(Debug)]
pub(crate) struct ContextExpansion {
    pub(crate) discovered_nodes: BTreeMap<String, DiscoveredNode>,
    pub(crate) reached_edges: BTreeMap<(u8, String, String), RepositoryContextEdgeState>,
    pub(crate) node_limit_reached: bool,
    pub(crate) relation_visit_limit_reached: bool,
}

pub(crate) fn normalize_context_query(query: RepositoryContextQuery) -> NormalizedContextQuery {
    let relation_kinds = query.relation_kinds.and_then(|mut kinds| {
        kinds.sort_by_key(relation_kind_order);
        kinds.dedup();
        (!kinds.is_empty()).then_some(kinds)
    });
    let max_depth = query.max_depth.min(MAX_CONTEXT_DEPTH);
    let max_nodes = query.max_nodes.min(MAX_CONTEXT_NODES);
    NormalizedContextQuery {
        query: query.query,
        direction: query.direction,
        relation_kinds,
        max_depth,
        max_nodes,
    }
}

pub(crate) type RelationAdjacency<'a> = BTreeMap<&'a str, Vec<&'a CodeRelationRecord>>;

pub(crate) fn relation_adjacency<'a>(
    relations: &'a [CodeRelationRecord],
) -> (RelationAdjacency<'a>, RelationAdjacency<'a>) {
    let mut outgoing = BTreeMap::new();
    let mut incoming = BTreeMap::new();
    for relation in relations {
        outgoing
            .entry(relation.source_record_id.as_str())
            .or_insert_with(Vec::new)
            .push(relation);
        incoming
            .entry(relation.target_record_id.as_str())
            .or_insert_with(Vec::new)
            .push(relation);
    }
    for relations in outgoing.values_mut().chain(incoming.values_mut()) {
        relations.sort_by(|left, right| relation_sort_key(left).cmp(&relation_sort_key(right)));
    }
    (outgoing, incoming)
}

pub(crate) fn relation_matches_kinds(
    filter: Option<&[CodeRelationKind]>,
    candidate: CodeRelationKind,
) -> bool {
    filter.is_none_or(|filter| filter.contains(&candidate))
}
fn relation_is_grounded(
    relation: &CodeRelationRecord,
    symbol_by_id: &BTreeMap<&str, &SymbolRecord>,
) -> bool {
    let Some(source) = symbol_by_id.get(relation.source_record_id.as_str()) else {
        return false;
    };
    let Some(target) = symbol_by_id.get(relation.target_record_id.as_str()) else {
        return false;
    };
    relation.source_provenance == source.provenance
        && relation.target_provenance == target.provenance
        && relation.parser_generation == source.provenance.parser_generation
        && relation.parser_generation == target.provenance.parser_generation
}

pub(crate) fn discover_reachable_node(
    record_id: String,
    depth: usize,
    seed_record_ids: &BTreeSet<String>,
    max_nodes: usize,
    discovered_nodes: &mut BTreeMap<String, DiscoveredNode>,
    frontier: &mut BTreeMap<String, BTreeSet<String>>,
    node_limit_reached: &mut bool,
) -> bool {
    let mut newly_reached = BTreeSet::new();
    if let Some(node) = discovered_nodes.get_mut(&record_id) {
        for seed_record_id in seed_record_ids {
            if node.seed_record_ids.insert(seed_record_id.clone()) {
                newly_reached.insert(seed_record_id.clone());
            }
        }
    } else {
        if discovered_nodes.len() >= max_nodes {
            *node_limit_reached = true;
            return false;
        }
        let mut seeds = BTreeSet::new();
        seeds.extend(seed_record_ids.iter().cloned());
        discovered_nodes.insert(
            record_id.clone(),
            DiscoveredNode {
                depth,
                seed_record_ids: seeds.clone(),
            },
        );
        newly_reached = seeds;
    }
    if !newly_reached.is_empty() {
        frontier.entry(record_id).or_default().extend(newly_reached);
    }
    true
}

pub(crate) fn record_edge(
    relation: &CodeRelationRecord,
    seed_record_ids: &BTreeSet<String>,
    depth: usize,
    reached_edges: &mut BTreeMap<(u8, String, String), RepositoryContextEdgeState>,
) {
    let key = (
        relation_kind_order(&relation.kind),
        relation.source_record_id.clone(),
        relation.target_record_id.clone(),
    );
    let state = reached_edges
        .entry(key)
        .or_insert_with(|| RepositoryContextEdgeState {
            relation: relation.clone(),
            depth,
            seed_record_ids: BTreeSet::new(),
        });
    state
        .seed_record_ids
        .extend(seed_record_ids.iter().cloned());
    state.depth = state.depth.min(depth);
}

pub(crate) fn relation_sort_key(
    relation: &CodeRelationRecord,
) -> (u8, &str, &str, &str, usize, usize, &str, usize, u16) {
    (
        relation_kind_order(&relation.kind),
        relation.source_record_id.as_str(),
        relation.target_record_id.as_str(),
        relation.source_provenance.file_path.as_str(),
        relation.source_provenance.source_range.start_line,
        relation.source_provenance.source_range.end_line,
        relation.target_provenance.file_path.as_str(),
        relation.target_provenance.source_range.start_line,
        relation.confidence_milli,
    )
}

pub(crate) fn relation_kind_order(kind: &CodeRelationKind) -> u8 {
    match kind {
        CodeRelationKind::Defines => 0,
        CodeRelationKind::Imports => 1,
        CodeRelationKind::Calls => 2,
        CodeRelationKind::Implements => 3,
        CodeRelationKind::Tests => 4,
    }
}
pub(crate) fn expand_context<'a>(
    query: &NormalizedContextQuery,
    seed_query: &QueryResult,
    symbol_by_id: &BTreeMap<&'a str, &'a SymbolRecord>,
    outgoing: &RelationAdjacency<'a>,
    incoming: &RelationAdjacency<'a>,
) -> ContextExpansion {
    let mut discovered_nodes = BTreeMap::new();
    let mut frontier = BTreeMap::new();
    let mut node_limit_reached = false;
    for (index, seed) in seed_query.records.iter().enumerate() {
        if index >= query.max_nodes {
            node_limit_reached = true;
            break;
        }
        if !symbol_by_id.contains_key(seed.record_id.as_str()) {
            continue;
        }
        let seed_ids = BTreeSet::from([seed.record_id.clone()]);
        discovered_nodes.insert(
            seed.record_id.clone(),
            DiscoveredNode {
                depth: 0,
                seed_record_ids: seed_ids.clone(),
            },
        );
        frontier.insert(seed.record_id.clone(), seed_ids);
    }
    node_limit_reached |=
        seed_query.summary.truncated || seed_query.records.len() > query.max_nodes;
    let mut reached_edges = BTreeMap::new();
    let mut relation_visits = 0;
    let mut relation_visit_limit_reached = false;
    for _ in 0..query.max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next_frontier = BTreeMap::new();
        for (record_id, seed_ids) in frontier {
            let Some(current) = discovered_nodes.get(&record_id) else {
                continue;
            };
            let depth = current.depth + 1;
            if depth > query.max_depth {
                continue;
            }
            let mut state = DirectionExpansion {
                query,
                symbol_by_id,
                seed_ids: &seed_ids,
                depth,
                discovered_nodes: &mut discovered_nodes,
                next_frontier: &mut next_frontier,
                reached_edges: &mut reached_edges,
                node_limit_reached: &mut node_limit_reached,
                relation_visits: &mut relation_visits,
                relation_visit_limit_reached: &mut relation_visit_limit_reached,
            };
            expand_direction(outgoing.get(record_id.as_str()), false, &mut state);
            expand_direction(incoming.get(record_id.as_str()), true, &mut state);
        }
        frontier = next_frontier;
    }
    ContextExpansion {
        discovered_nodes,
        reached_edges,
        node_limit_reached,
        relation_visit_limit_reached,
    }
}

struct DirectionExpansion<'a, 'b> {
    query: &'a NormalizedContextQuery,
    symbol_by_id: &'b BTreeMap<&'b str, &'b SymbolRecord>,
    seed_ids: &'a BTreeSet<String>,
    depth: usize,
    discovered_nodes: &'a mut BTreeMap<String, DiscoveredNode>,
    next_frontier: &'a mut BTreeMap<String, BTreeSet<String>>,
    reached_edges: &'a mut BTreeMap<(u8, String, String), RepositoryContextEdgeState>,
    node_limit_reached: &'a mut bool,
    relation_visits: &'a mut usize,
    relation_visit_limit_reached: &'a mut bool,
}

fn expand_direction<'b>(
    relations: Option<&Vec<&'b CodeRelationRecord>>,
    incoming: bool,
    state: &mut DirectionExpansion<'_, 'b>,
) {
    if !direction_enabled(state.query.direction, incoming) {
        return;
    }
    let Some(relations) = relations else {
        return;
    };
    for relation in relations {
        if *state.relation_visits >= MAX_CONTEXT_RELATION_VISITS {
            *state.relation_visit_limit_reached = true;
            return;
        }
        *state.relation_visits += 1;
        if !relation_matches_kinds(state.query.relation_kinds.as_deref(), relation.kind)
            || !relation_is_grounded(relation, state.symbol_by_id)
        {
            continue;
        }
        let next_record_id = if incoming {
            relation.source_record_id.clone()
        } else {
            relation.target_record_id.clone()
        };
        if discover_reachable_node(
            next_record_id,
            state.depth,
            state.seed_ids,
            state.query.max_nodes,
            state.discovered_nodes,
            state.next_frontier,
            state.node_limit_reached,
        ) {
            record_edge(relation, state.seed_ids, state.depth, state.reached_edges);
        }
    }
}

fn direction_enabled(direction: ContextDirection, incoming: bool) -> bool {
    matches!(
        (direction, incoming),
        (ContextDirection::Incoming, true)
            | (ContextDirection::Outgoing, false)
            | (ContextDirection::Both, _)
    )
}
