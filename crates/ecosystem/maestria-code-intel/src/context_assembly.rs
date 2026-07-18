use crate::CodeRelationSummary;
use crate::context::{
    RepositoryContextEdge, RepositoryContextNode, RepositoryContextResult, RepositoryContextSummary,
};
use crate::context_support::{
    ContextExpansion, NormalizedContextQuery, RepositoryContextEdgeState,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct ContextAssembly<'a> {
    pub(crate) query: NormalizedContextQuery,
    pub(crate) seed_query: crate::QueryResult,
    pub(crate) symbol_by_id: &'a BTreeMap<&'a str, &'a crate::SymbolRecord>,
    pub(crate) expansion: ContextExpansion,
    pub(crate) relation_summary: CodeRelationSummary,
}

pub(crate) fn assemble_context_result(input: ContextAssembly<'_>) -> RepositoryContextResult {
    let ContextAssembly {
        query,
        seed_query,
        symbol_by_id,
        expansion,
        relation_summary,
    } = input;
    let ContextExpansion {
        discovered_nodes,
        reached_edges,
        node_limit_reached,
        relation_visit_limit_reached,
    } = expansion;
    let mut node_ids = discovered_nodes.keys().cloned().collect::<Vec<_>>();
    node_ids.sort_by(|left, right| {
        node_sort_key(symbol_by_id[left.as_str()], symbol_by_id[right.as_str()])
    });
    let matched_nodes = node_ids.len();
    let nodes = node_ids
        .iter()
        .map(|record_id| {
            let mut seed_record_ids = discovered_nodes[record_id]
                .seed_record_ids
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            seed_record_ids.sort_unstable();
            RepositoryContextNode {
                record: symbol_by_id[record_id.as_str()].clone(),
                depth: discovered_nodes[record_id].depth,
                seed_record_ids,
            }
        })
        .collect::<Vec<_>>();
    let selected_nodes = nodes
        .iter()
        .map(|node| node.record.record_id.clone())
        .collect::<BTreeSet<_>>();
    let matched_edges = reached_edges.len();
    let mut edges = reached_edges
        .into_values()
        .filter_map(|edge| build_context_edge(edge, &selected_nodes))
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| context_edge_sort_key(left).cmp(&context_edge_sort_key(right)));
    let returned_edges = edges.len();
    let summary = RepositoryContextSummary {
        seed_query: seed_query.summary,
        direction: query.direction,
        relation_kinds: query.relation_kinds,
        max_depth: query.max_depth,
        max_nodes: query.max_nodes,
        matched_nodes,
        returned_nodes: nodes.len(),
        matched_edges,
        returned_edges,
        nodes_truncated: node_limit_reached,
        edges_truncated: matched_edges > returned_edges
            || node_limit_reached
            || relation_visit_limit_reached,
        relation_summary,
    };
    RepositoryContextResult {
        summary,
        nodes,
        edges,
    }
}

fn build_context_edge(
    edge: RepositoryContextEdgeState,
    selected_nodes: &BTreeSet<String>,
) -> Option<RepositoryContextEdge> {
    if !selected_nodes.contains(&edge.relation.source_record_id)
        || !selected_nodes.contains(&edge.relation.target_record_id)
    {
        return None;
    }
    let mut seed_record_ids = edge.seed_record_ids.into_iter().collect::<Vec<_>>();
    seed_record_ids.sort_unstable();
    Some(RepositoryContextEdge {
        relation: edge.relation,
        depth: edge.depth,
        seed_record_ids,
    })
}

fn context_edge_sort_key(
    edge: &RepositoryContextEdge,
) -> (u8, &str, &str, &str, usize, usize, &str, usize, usize, u16) {
    let relation = &edge.relation;
    (
        relation_kind_order(&relation.kind),
        relation.source_record_id.as_str(),
        relation.target_record_id.as_str(),
        relation.source_provenance.file_path.as_str(),
        relation.source_provenance.source_range.start_line,
        relation.source_provenance.source_range.end_line,
        relation.target_provenance.file_path.as_str(),
        relation.target_provenance.source_range.start_line,
        edge.depth,
        relation.confidence_milli,
    )
}

fn node_sort_key(left: &crate::SymbolRecord, right: &crate::SymbolRecord) -> std::cmp::Ordering {
    (
        left.provenance.file_path.as_str(),
        left.provenance.source_range.start_line,
        left.provenance.source_range.end_line,
        left.qualified_name.as_str(),
    )
        .cmp(&(
            right.provenance.file_path.as_str(),
            right.provenance.source_range.start_line,
            right.provenance.source_range.end_line,
            right.qualified_name.as_str(),
        ))
}

fn relation_kind_order(kind: &crate::CodeRelationKind) -> u8 {
    match kind {
        crate::CodeRelationKind::Defines => 0,
        crate::CodeRelationKind::Imports => 1,
        crate::CodeRelationKind::Calls => 2,
        crate::CodeRelationKind::Implements => 3,
        crate::CodeRelationKind::Tests => 4,
    }
}
