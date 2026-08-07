//! Candidate-to-relation resolution for all backends.
//!
//! Rust candidates resolve scope-aware paths (`crate::`, `self::`,
//! `super::`, imports); Python candidates resolve exact qualified names and
//! unambiguous short names (see `python`).

use super::python;
use super::relation::relation_for;
use crate::symbols::RelationCandidate;
use crate::{CodeRelationKind, CodeRelationRecord, SymbolKind, SymbolRecord};
use std::collections::BTreeMap;

pub(super) fn resolve_candidate(
    parser_generation: &str,
    by_id: &BTreeMap<String, &SymbolRecord>,
    by_qualified_name: &BTreeMap<String, Vec<&SymbolRecord>>,
    by_name: &BTreeMap<String, Vec<&SymbolRecord>>,
    candidate: &crate::symbols::RelationCandidate,
) -> Vec<CodeRelationRecord> {
    match candidate {
        RelationCandidate::Defines {
            source_module_qualified,
            target_record_id,
        } => {
            let target = by_id.get(target_record_id).copied();
            let source = target.and_then(|target| {
                by_qualified_name
                    .get(source_module_qualified)
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
            by_id.get(source_record_id).copied(),
            resolve_target(by_qualified_name, target_qualified, None, None),
        )
        .into_iter()
        .collect(),
        RelationCandidate::Calls {
            source_record_id,
            source_qualified,
            module_scope,
            target_path,
            self_receiver,
        } => {
            let source = by_id.get(source_record_id).copied();
            let target = if *self_receiver {
                resolve_self_receiver_target(by_qualified_name, source_qualified, target_path)
            } else {
                resolve_target(
                    by_qualified_name,
                    target_path,
                    Some(source_qualified),
                    Some(module_scope),
                )
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
            by_id.get(source_record_id).copied(),
            resolve_target(by_qualified_name, target_qualified, None, None),
        )
        .into_iter()
        .collect(),
        RelationCandidate::PythonCall {
            source_record_id,
            target_hint,
        } => relation_for(
            parser_generation,
            CodeRelationKind::Calls,
            by_id.get(source_record_id).copied(),
            python::resolve_call(by_qualified_name, by_name, target_hint),
        )
        .into_iter()
        .collect(),
    }
}

fn resolve_target<'a>(
    by_qualified_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    path: &str,
    source_qualified: Option<&str>,
    module_scope: Option<&str>,
) -> Option<&'a SymbolRecord> {
    resolve_target_with_depth(by_qualified_name, path, source_qualified, module_scope, 0)
}

fn resolve_target_with_depth<'a>(
    by_qualified_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    path: &str,
    source_qualified: Option<&str>,
    module_scope: Option<&str>,
    depth: usize,
) -> Option<&'a SymbolRecord> {
    if depth > 2 {
        return None;
    }
    if let Some(target) = resolve_import_prefix(
        by_qualified_name,
        path,
        source_qualified,
        module_scope,
        depth + 1,
    ) {
        return Some(target);
    }
    for candidate in
        super::relation_paths::relation_candidate_names(path, source_qualified, module_scope)
    {
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
            if let Some(target) = resolve_import_matches(
                by_qualified_name,
                matches,
                source_qualified,
                module_scope,
                depth,
            ) {
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
    module_scope: Option<&str>,
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
    resolve_target_with_depth(
        by_qualified_name,
        imported,
        source_qualified,
        module_scope,
        depth + 1,
    )
}
fn resolve_import_prefix<'a>(
    by_qualified_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    path: &str,
    source_qualified: Option<&str>,
    module_scope: Option<&str>,
    depth: usize,
) -> Option<&'a SymbolRecord> {
    let (prefix, remainder) = path.split_once("::")?;
    for prefix_candidate in
        super::relation_paths::relation_candidate_names(prefix, source_qualified, module_scope)
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
        if let Some(target) = resolve_target_with_depth(
            by_qualified_name,
            &expanded,
            source_qualified,
            module_scope,
            depth,
        ) {
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

pub(super) fn unique_symbol<'a>(matches: &[&'a SymbolRecord]) -> Option<&'a SymbolRecord> {
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
