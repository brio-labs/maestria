use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    CodeRelationKind, CodeRelationRecord, CodeRelationSummary, RelationSourceAvailability,
    RelationSourceKind, RelationSourceStatus, SymbolRecord,
};

pub(crate) const AST_RELATION_CONFIDENCE_MILLI: u16 = 1000;
pub(crate) const LSP_DEGRADED_REASON: &str =
    "rust-analyzer-backed relation extraction is unavailable in this build";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum RelationCandidate {
    Defines {
        source_module_qualified: String,
        target_record_id: String,
    },
    Imports {
        source_record_id: String,
        target_qualified: String,
    },
    Calls {
        source_record_id: String,
        source_qualified: String,
        /// Enclosing module of the source symbol (module stack joined by `::`),
        /// used to resolve `self::`, `super::`, and bare paths lexically.
        module_scope: String,
        target_path: String,
        self_receiver: bool,
    },
    Implements {
        source_record_id: String,
        target_qualified: String,
    },
    /// Python call: the bare or dotted callee expression as written (e.g.
    /// `get`, `requests.get`, `self.helper`). Resolution is exact against
    /// qualified names first, then the short name when unambiguous.
    PythonCall {
        source_record_id: String,
        target_hint: String,
    },
}

pub(crate) fn relation_status_summary(total_relations: usize) -> CodeRelationSummary {
    CodeRelationSummary {
        total_relations,
        source_statuses: vec![
            RelationSourceStatus {
                source: RelationSourceKind::Ast,
                availability: RelationSourceAvailability::Available,
                reason: None,
            },
            RelationSourceStatus {
                source: RelationSourceKind::RustAnalyzer,
                availability: RelationSourceAvailability::Degraded,
                reason: Some(LSP_DEGRADED_REASON.to_string()),
            },
        ],
    }
}

pub(crate) fn resolve_relations(
    parser_generation: &str,
    symbols: &[SymbolRecord],
    candidates: &[RelationCandidate],
) -> Vec<CodeRelationRecord> {
    let mut by_id = BTreeMap::<String, &SymbolRecord>::new();
    let mut by_qualified_name = BTreeMap::<String, Vec<&SymbolRecord>>::new();
    let mut by_name = BTreeMap::<String, Vec<&SymbolRecord>>::new();
    for symbol in symbols {
        by_id.insert(symbol.record_id.clone(), symbol);
        by_qualified_name
            .entry(symbol.qualified_name.clone())
            .or_default()
            .push(symbol);
        by_name.entry(symbol.name.clone()).or_default().push(symbol);
    }
    for list in by_qualified_name.values_mut() {
        list.sort_by_key(|record| {
            (
                record.provenance.file_path.as_str(),
                record.provenance.source_range.start_line(),
                record.provenance.source_range.end_line(),
                record.record_id.as_str(),
            )
        });
    }
    let mut relations = candidates
        .iter()
        .flat_map(|candidate| {
            super::resolution::resolve_candidate(
                parser_generation,
                &by_id,
                &by_qualified_name,
                &by_name,
                candidate,
            )
        })
        .collect::<Vec<_>>();
    relations.sort_by(relation_order);
    relations.dedup_by(|left, right| relation_order(left, right).is_eq());
    relations
}

pub(super) fn relation_for(
    parser_generation: &str,
    kind: CodeRelationKind,
    source: Option<&SymbolRecord>,
    target: Option<&SymbolRecord>,
) -> Option<CodeRelationRecord> {
    match (source, target) {
        (Some(source), Some(target)) => {
            Some(make_relation(parser_generation, kind, source, target))
        }
        _ => None,
    }
}

fn make_relation(
    parser_generation: &str,
    kind: CodeRelationKind,
    source: &SymbolRecord,
    target: &SymbolRecord,
) -> CodeRelationRecord {
    CodeRelationRecord {
        kind,
        source_record_id: source.record_id.clone(),
        target_record_id: target.record_id.clone(),
        source_provenance: source.provenance.clone(),
        target_provenance: target.provenance.clone(),
        parser_generation: crate::types::ParserGeneration::new(parser_generation),
        confidence_milli: AST_RELATION_CONFIDENCE_MILLI,
        source_kind: RelationSourceKind::Ast,
    }
}

fn relation_order(left: &CodeRelationRecord, right: &CodeRelationRecord) -> std::cmp::Ordering {
    relation_key(left).cmp(&relation_key(right))
}

fn relation_key(
    record: &CodeRelationRecord,
) -> (u8, &str, &str, &str, usize, usize, &str, usize, u16) {
    (
        relation_kind_order(&record.kind),
        record.source_record_id.as_str(),
        record.target_record_id.as_str(),
        record.source_provenance.file_path.as_str(),
        record.source_provenance.source_range.start_line(),
        record.source_provenance.source_range.end_line(),
        record.target_provenance.file_path.as_str(),
        record.target_provenance.source_range.start_line(),
        record.confidence_milli,
    )
}

fn relation_kind_order(kind: &CodeRelationKind) -> u8 {
    match kind {
        CodeRelationKind::Defines => 0,
        CodeRelationKind::Imports => 1,
        CodeRelationKind::Calls => 2,
        CodeRelationKind::Implements => 3,
        CodeRelationKind::Tests => 4,
    }
}
