use crate::common::*;
use maestria_code_intel::*;
use std::collections::BTreeMap;
use std::error::Error;

fn assert_relation_status(index: &RepositoryCodeIndex) {
    let mut has_ast = false;
    let mut has_rust_analyzer_degraded = false;
    for status in &index.summary.relation_summary.source_statuses {
        match status.source {
            RelationSourceKind::Ast => {
                has_ast = status.availability == RelationSourceAvailability::Available;
            }
            RelationSourceKind::RustAnalyzer => {
                if status.availability == RelationSourceAvailability::Degraded {
                    has_rust_analyzer_degraded = status
                        .reason
                        .as_ref()
                        .is_some_and(|reason| reason.contains("rust-analyzer"));
                }
            }
        }
    }
    assert!(has_ast);
    assert!(has_rust_analyzer_degraded);
}

fn assert_relation_provenance(index: &RepositoryCodeIndex) {
    let symbol_ids = index
        .symbols
        .iter()
        .map(|symbol| (symbol.record_id.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    for relation in &index.relations {
        assert!(symbol_ids.contains_key(relation.source_record_id.as_str()));
        assert!(symbol_ids.contains_key(relation.target_record_id.as_str()));
        assert_eq!(
            relation.source_provenance,
            symbol_ids[relation.source_record_id.as_str()].provenance
        );
        assert_eq!(
            relation.target_provenance,
            symbol_ids[relation.target_record_id.as_str()].provenance
        );
        assert_eq!(relation.source_kind, RelationSourceKind::Ast);
        assert_eq!(relation.parser_generation, index.summary.parser_generation);
    }
}

fn assert_definition_and_import_relations(
    index: &RepositoryCodeIndex,
) -> Result<(), Box<dyn Error>> {
    let symbol_ids = index
        .symbols
        .iter()
        .map(|symbol| (symbol.record_id.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    let by_name = index
        .symbols
        .iter()
        .map(|symbol| (symbol.qualified_name.as_str(), symbol.record_id.as_str()))
        .collect::<BTreeMap<_, _>>();
    let add = symbol_id(&index.symbols, "math::add")?;
    let notifier = index
        .symbols
        .iter()
        .find(|symbol| {
            symbol.qualified_name == "Notifier" && matches!(symbol.kind, SymbolKind::Trait)
        })
        .map(|symbol| symbol.record_id.as_str())
        .ok_or("missing Notifier trait symbol")?;
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Implements
            && relation.target_record_id == notifier
            && symbol_ids
                .get(relation.source_record_id.as_str())
                .is_some_and(|symbol| symbol.qualified_name == "Widget")
    }));
    let math_symbol = by_name.get("math").ok_or("missing math module symbol")?;
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Defines
            && relation.source_record_id == *math_symbol
            && relation.target_record_id == add
    }));
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Imports
            && symbol_ids[relation.target_record_id.as_str()]
                .qualified_name
                .ends_with("math::add")
    }));
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Imports
            && symbol_ids[relation.target_record_id.as_str()]
                .qualified_name
                .ends_with("Notifier")
    }));
    Ok(())
}

fn assert_call_and_test_relations(index: &RepositoryCodeIndex) -> Result<(), Box<dyn Error>> {
    let add = symbol_id(&index.symbols, "math::add")?;
    let from_helpers = symbol_id(&index.symbols, "helpers::from_helpers")?;
    let imported_add_usage = symbol_id(&index.symbols, "imported_add_usage")?;
    let alias_usage = symbol_id(&index.symbols, "alias_usage")?;
    let helpers_call = symbol_id(&index.symbols, "helpers_call")?;
    let widget_helper = symbol_id(&index.symbols, "Widget::helper")?;
    let widget_calls_self = symbol_id(&index.symbols, "Widget::calls_self")?;
    let test_relation = symbol_id(&index.symbols, "tests::relations_are_seen")?;
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == imported_add_usage
            && relation.target_record_id == add
    }));
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == alias_usage
            && relation.target_record_id == add
    }));
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == widget_calls_self
            && relation.target_record_id == widget_helper
    }));
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == helpers_call
            && relation.target_record_id == from_helpers
    }));
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == test_relation
            && relation.target_record_id == add
    }));
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Tests
            && relation.source_record_id == test_relation
            && relation.target_record_id == add
    }));
    Ok(())
}

fn assert_scope_aware_calls(index: &RepositoryCodeIndex) -> Result<(), Box<dyn Error>> {
    let add = symbol_id(&index.symbols, "math::add")?;
    let target = symbol_id(&index.symbols, "target")?;
    let module_alias_usage = symbol_id(&index.symbols, "module_alias_usage")?;
    let shadowed_call = symbol_id(&index.symbols, "shadowed_call")?;
    let initializer_call = symbol_id(&index.symbols, "initializer_call")?;
    let scope_leak_call = symbol_id(&index.symbols, "scope_leak_call")?;
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == module_alias_usage
            && relation.target_record_id == add
    }));
    assert!(!index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == shadowed_call
            && relation.target_record_id == target
    }));
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == initializer_call
            && relation.target_record_id == target
    }));
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == scope_leak_call
            && relation.target_record_id == target
    }));
    Ok(())
}

fn assert_condition_scope_calls(index: &RepositoryCodeIndex) -> Result<(), Box<dyn Error>> {
    let target = symbol_id(&index.symbols, "target")?;
    for source_name in ["if_let_scope_call", "while_let_scope_call"] {
        let source = symbol_id(&index.symbols, source_name)?;
        assert!(index.relations.iter().any(|relation| {
            relation.kind == CodeRelationKind::Calls
                && relation.source_record_id == source
                && relation.target_record_id == target
        }));
    }
    Ok(())
}

fn assert_containing_scope_calls(index: &RepositoryCodeIndex) -> Result<(), Box<dyn Error>> {
    let helper = symbol_id(&index.symbols, "nested::helper")?;
    for source_name in ["nested::caller", "nested::self_caller"] {
        let source = symbol_id(&index.symbols, source_name)?;
        assert!(index.relations.iter().any(|relation| {
            relation.kind == CodeRelationKind::Calls
                && relation.source_record_id == source
                && relation.target_record_id == helper
        }));
    }
    let target = symbol_id(&index.symbols, "target")?;
    let root_source = symbol_id(&index.symbols, "self_root_call")?;
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == root_source
            && relation.target_record_id == target
    }));
    let module_helper = symbol_id(&index.symbols, "method_scope::helper")?;
    let method_caller = symbol_id(&index.symbols, "method_scope::Widget::caller")?;
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == method_caller
            && relation.target_record_id == module_helper
    }));
    let type_helper = symbol_id(&index.symbols, "method_scope::Widget::helper")?;
    let self_type_caller = symbol_id(&index.symbols, "method_scope::Widget::self_type_caller")?;
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id == self_type_caller
            && relation.target_record_id == type_helper
    }));
    Ok(())
}

#[test]
fn ast_relation_graph_emits_only_grounded_and_sorted_edges() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace_with_relations()?;
    let index = build_index(tmp.path(), "g4")?;
    assert_relation_status(&index);
    assert_relation_provenance(&index);
    assert_definition_and_import_relations(&index)?;
    assert_call_and_test_relations(&index)?;
    assert_scope_aware_calls(&index)?;

    assert_condition_scope_calls(&index)?;
    assert_containing_scope_calls(&index)?;
    let mut sorted = index.relations.clone();
    sorted.sort_by(|left, right| relation_sort_key(left).cmp(&relation_sort_key(right)));
    assert_eq!(sorted, index.relations);
    Ok(())
}
