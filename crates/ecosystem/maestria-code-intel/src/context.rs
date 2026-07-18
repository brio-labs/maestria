use crate::context_assembly::{ContextAssembly, assemble_context_result};
use crate::context_support::{expand_context, normalize_context_query, relation_adjacency};
use crate::symbols;
use crate::{
    CodeQuery, CodeRelationKind, CodeRelationRecord, CodeRelationSummary, QuerySummary,
    RepositoryCodeIndex, SymbolRecord,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const MAX_CONTEXT_DEPTH: usize = 16;
pub(crate) const MAX_CONTEXT_NODES: usize = 1_024;
pub(crate) const MAX_CONTEXT_SEED_MATCHES: usize = 1_000;
pub(crate) const DEFAULT_CONTEXT_DEPTH: usize = 2;
pub(crate) const DEFAULT_CONTEXT_NODES: usize = 64;
pub(crate) const MAX_CONTEXT_RELATION_VISITS: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextDirection {
    Outgoing,
    Incoming,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryContextQuery {
    pub query: CodeQuery,
    pub direction: ContextDirection,
    pub relation_kinds: Option<Vec<CodeRelationKind>>,
    pub max_depth: usize,
    pub max_nodes: usize,
}

impl Default for RepositoryContextQuery {
    fn default() -> Self {
        Self {
            query: CodeQuery::All,
            direction: ContextDirection::Both,
            relation_kinds: None,
            max_depth: DEFAULT_CONTEXT_DEPTH,
            max_nodes: DEFAULT_CONTEXT_NODES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryContextNode {
    pub record: SymbolRecord,
    pub depth: usize,
    pub seed_record_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryContextEdge {
    pub relation: CodeRelationRecord,
    pub depth: usize,
    pub seed_record_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryContextSummary {
    pub seed_query: QuerySummary,
    pub direction: ContextDirection,
    pub relation_kinds: Option<Vec<CodeRelationKind>>,
    pub max_depth: usize,
    pub max_nodes: usize,
    pub matched_nodes: usize,
    pub returned_nodes: usize,
    pub matched_edges: usize,
    pub returned_edges: usize,
    pub nodes_truncated: bool,
    pub edges_truncated: bool,
    pub relation_summary: CodeRelationSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryContextResult {
    pub summary: RepositoryContextSummary,
    pub nodes: Vec<RepositoryContextNode>,
    pub edges: Vec<RepositoryContextEdge>,
}

impl RepositoryCodeIndex {
    /// Build a bounded, deterministic context graph from matching seed symbols.
    pub fn context(&self, query: RepositoryContextQuery) -> RepositoryContextResult {
        let query = normalize_context_query(query);
        let seed_query =
            symbols::query_symbols(&self.symbols, query.query.clone(), MAX_CONTEXT_SEED_MATCHES);
        let symbol_by_id = self
            .symbols
            .iter()
            .map(|symbol| (symbol.record_id.as_str(), symbol))
            .collect::<BTreeMap<_, _>>();
        let (outgoing, incoming) = relation_adjacency(&self.relations);
        let expansion = expand_context(&query, &seed_query, &symbol_by_id, &outgoing, &incoming);
        assemble_context_result(ContextAssembly {
            query,
            seed_query,
            symbol_by_id: &symbol_by_id,
            expansion,
            relation_summary: self.summary.relation_summary.clone(),
        })
    }
}
