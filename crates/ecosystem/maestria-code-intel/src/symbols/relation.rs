use std::collections::BTreeMap;

use crate::{
    CodeRelationKind, CodeRelationRecord, CodeRelationSummary, RelationSourceAvailability,
    RelationSourceKind, RelationSourceStatus, SymbolKind, SymbolRecord,
};

pub(crate) const AST_RELATION_CONFIDENCE_MILLI: u16 = 1000;
pub(crate) const LSP_DEGRADED_REASON: &str =
    "rust-analyzer-backed relation extraction is unavailable in this build";

#[derive(Debug)]
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
        target_path: String,
        self_receiver: bool,
    },
    Implements {
        source_record_id: String,
        target_qualified: String,
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
    candidates: Vec<RelationCandidate>,
) -> Vec<CodeRelationRecord> {
    let mut by_id = BTreeMap::<String, &SymbolRecord>::new();
    let mut by_qualified_name = BTreeMap::<String, Vec<&SymbolRecord>>::new();
    for symbol in symbols {
        by_id.insert(symbol.record_id.clone(), symbol);
        by_qualified_name
            .entry(symbol.qualified_name.clone())
            .or_default()
            .push(symbol);
    }
    for list in by_qualified_name.values_mut() {
        list.sort_by_key(|record| {
            (
                record.provenance.file_path.as_str(),
                record.provenance.source_range.start_line,
                record.provenance.source_range.end_line,
                record.record_id.as_str(),
            )
        });
    }
    let mut relations = candidates
        .into_iter()
        .flat_map(|candidate| {
            resolve_candidate(parser_generation, &by_id, &by_qualified_name, candidate)
        })
        .collect::<Vec<_>>();
    relations.sort_by(relation_order);
    relations.dedup_by(|left, right| relation_order(left, right).is_eq());
    relations
}

fn resolve_candidate(
    parser_generation: &str,
    by_id: &BTreeMap<String, &SymbolRecord>,
    by_qualified_name: &BTreeMap<String, Vec<&SymbolRecord>>,
    candidate: RelationCandidate,
) -> Vec<CodeRelationRecord> {
    match candidate {
        RelationCandidate::Defines {
            source_module_qualified,
            target_record_id,
        } => {
            let target = by_id.get(&target_record_id).copied();
            let source = target.and_then(|target| {
                by_qualified_name
                    .get(&source_module_qualified)
                    .and_then(|matches| resolve_definition_source(matches, target))
            });
            relation_for(parser_generation, CodeRelationKind::Defines, source, target)
                .into_iter()
                .collect()
        }
        RelationCandidate::Imports {
            source_record_id,
            target_qualified,
        } => relation_for(
            parser_generation,
            CodeRelationKind::Imports,
            by_id.get(&source_record_id).copied(),
            resolve_target(by_qualified_name, &target_qualified, None),
        )
        .into_iter()
        .collect(),
        RelationCandidate::Calls {
            source_record_id,
            source_qualified,
            target_path,
            self_receiver,
        } => {
            let source = by_id.get(&source_record_id).copied();
            let target = if self_receiver {
                resolve_self_receiver_target(by_qualified_name, &source_qualified, &target_path)
            } else {
                resolve_target(by_qualified_name, &target_path, Some(&source_qualified))
            };
            let Some(call) =
                relation_for(parser_generation, CodeRelationKind::Calls, source, target)
            else {
                return Vec::new();
            };
            let mut relations = vec![call];
            if let Some(source) = source
                && source.is_test
                && let Some(test_relation) = relation_for(
                    parser_generation,
                    CodeRelationKind::Tests,
                    Some(source),
                    target,
                )
            {
                relations.push(test_relation);
            }
            relations
        }
        RelationCandidate::Implements {
            source_record_id,
            target_qualified,
        } => relation_for(
            parser_generation,
            CodeRelationKind::Implements,
            by_id.get(&source_record_id).copied(),
            resolve_target(by_qualified_name, &target_qualified, None),
        )
        .into_iter()
        .collect(),
    }
}

fn relation_for(
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
        parser_generation: parser_generation.to_string(),
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
        record.source_provenance.source_range.start_line,
        record.source_provenance.source_range.end_line,
        record.target_provenance.file_path.as_str(),
        record.target_provenance.source_range.start_line,
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

fn resolve_target<'a>(
    by_qualified_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    path: &str,
    source_qualified: Option<&str>,
) -> Option<&'a SymbolRecord> {
    resolve_target_with_depth(by_qualified_name, path, source_qualified, 0)
}

fn resolve_target_with_depth<'a>(
    by_qualified_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    path: &str,
    source_qualified: Option<&str>,
    depth: usize,
) -> Option<&'a SymbolRecord> {
    if depth > 2 {
        return None;
    }
    if let Some(target) =
        resolve_import_prefix(by_qualified_name, path, source_qualified, depth + 1)
    {
        return Some(target);
    }
    for candidate in super::relation_paths::relation_candidate_names(path, source_qualified) {
        let Some(matches) = by_qualified_name.get(&candidate) else {
            continue;
        };
        if path.starts_with("crate::")
            && let Some(symbol) = unique_symbol(matches)
            && symbol.kind != SymbolKind::Import
        {
            return Some(symbol);
        }
        if matches
            .iter()
            .any(|symbol| symbol.kind == SymbolKind::Import)
        {
            if let Some(target) =
                resolve_import_matches(by_qualified_name, matches, source_qualified, depth)
            {
                return Some(target);
            }
            continue;
        }
        if let Some(symbol) = unique_symbol(matches) {
            return Some(symbol);
        }
    }
    None
}

fn resolve_import_matches<'a>(
    by_qualified_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    matches: &[&'a SymbolRecord],
    source_qualified: Option<&str>,
    depth: usize,
) -> Option<&'a SymbolRecord> {
    let mut imports = matches
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import);
    let import = match (imports.next(), imports.next()) {
        (Some(import), None) => *import,
        _ => return None,
    };
    let imported = import.imports.first()?;
    let imported = imported
        .split_once(" as ")
        .map_or(imported.as_str(), |(target, _)| target);
    resolve_target_with_depth(by_qualified_name, imported, source_qualified, depth + 1)
}
fn resolve_import_prefix<'a>(
    by_qualified_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    path: &str,
    source_qualified: Option<&str>,
    depth: usize,
) -> Option<&'a SymbolRecord> {
    let (prefix, remainder) = path.split_once("::")?;
    for prefix_candidate in
        super::relation_paths::relation_candidate_names(prefix, source_qualified)
    {
        let Some(matches) = by_qualified_name.get(&prefix_candidate) else {
            continue;
        };
        let Some(import) = unique_symbol(matches) else {
            continue;
        };
        if import.kind != SymbolKind::Import {
            continue;
        }
        let Some(imported) = import.imports.first() else {
            continue;
        };
        let imported = imported
            .split_once(" as ")
            .map_or(imported.as_str(), |(target, _)| target);
        let expanded = format!("{imported}::{remainder}");
        if let Some(target) =
            resolve_target_with_depth(by_qualified_name, &expanded, source_qualified, depth)
        {
            return Some(target);
        }
    }
    None
}
fn resolve_definition_source<'a>(
    matches: &[&'a SymbolRecord],
    target: &SymbolRecord,
) -> Option<&'a SymbolRecord> {
    let scoped = matches
        .iter()
        .copied()
        .filter(|symbol| symbol.package == target.package && symbol.target == target.target)
        .collect::<Vec<_>>();
    if matches!(&target.kind, SymbolKind::Method) {
        if let Some(source) = unique_kind(&scoped, &SymbolKind::Impl) {
            return Some(source);
        }
        if let Some(source) = unique_kind(&scoped, &SymbolKind::Trait) {
            return Some(source);
        }
    } else if let Some(source) = unique_kind(&scoped, &SymbolKind::Module) {
        return Some(source);
    }
    unique_symbol(&scoped)
}

fn unique_kind<'a>(matches: &[&'a SymbolRecord], kind: &SymbolKind) -> Option<&'a SymbolRecord> {
    let mut candidates = matches
        .iter()
        .copied()
        .filter(|symbol| &symbol.kind == kind);
    match (candidates.next(), candidates.next()) {
        (Some(symbol), None) => Some(symbol),
        _ => None,
    }
}

fn unique_symbol<'a>(matches: &[&'a SymbolRecord]) -> Option<&'a SymbolRecord> {
    let mut declarations = matches
        .iter()
        .filter(|symbol| symbol.kind != SymbolKind::Import);
    match (declarations.next(), declarations.next()) {
        (Some(symbol), None) => Some(*symbol),
        (None, None) if matches.len() == 1 => matches.first().copied(),
        _ => None,
    }
}

fn resolve_self_receiver_target<'a>(
    by_qualified_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    source_qualified: &str,
    method: &str,
) -> Option<&'a SymbolRecord> {
    let candidate = source_qualified
        .rsplit_once("::")
        .map(|(parent, _)| format!("{parent}::{method}"))?;
    by_qualified_name
        .get(&candidate)
        .and_then(|candidates| unique_symbol(candidates))
}
