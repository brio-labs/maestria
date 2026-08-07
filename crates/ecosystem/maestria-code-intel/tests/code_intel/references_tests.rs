//! Cross-file references queries: inbound usage sites, outbound targets,
//! direction filters, bounds, authorization, and incremental equivalence.

use super::common::{assert_equivalent_to_full_rebuild, build_or_update, init_git, write_file};
use maestria_code_intel::*;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use tempfile::tempdir;

fn authorize_all() -> impl FnMut(&SymbolRecord) -> Result<bool, Box<dyn Error>> {
    |_: &SymbolRecord| Ok(true)
}

/// Workspace with `add` defined in module `calc` and call sites in sibling
/// module files `b` and `c` (plus the `use` imports that resolve them).
fn make_workspace_with_cross_file_calls() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let root = tempdir()?;
    write_file(
        &root.path().join("Cargo.toml"),
        r#"
[workspace]
members = ["app"]

[workspace.package]
edition = "2024"
"#,
    )?;
    fs::create_dir_all(root.path().join("app/src"))?;
    write_file(
        &root.path().join("app/Cargo.toml"),
        r#"
[package]
name = "app"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        &root.path().join("app/src/lib.rs"),
        "pub mod calc;\npub mod b;\npub mod c;\n",
    )?;
    write_file(
        &root.path().join("app/src/calc.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )?;
    write_file(
        &root.path().join("app/src/b.rs"),
        "use crate::calc::add;\npub fn b_call() -> i32 { add(1, 2) }\n",
    )?;
    write_file(
        &root.path().join("app/src/c.rs"),
        "use crate::calc::add;\npub fn c_call() -> i32 { add(3, 4) }\n",
    )?;
    init_git(root.path())?;
    Ok(root)
}

fn references(
    index: &RepositoryCodeIndex,
    pattern: &str,
    direction: ReferencesDirection,
    limit: usize,
) -> Result<QueryResult, Box<dyn Error>> {
    index.references(
        CodeQuery::References {
            pattern: pattern.to_string(),
            direction,
        },
        limit,
        authorize_all(),
    )
}

fn assert_relations_grounded(index: &RepositoryCodeIndex, result: &QueryResult) {
    let symbol_ids = index
        .symbols
        .iter()
        .map(|symbol| (symbol.record_id.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    for relation in &result.relations {
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
        assert_eq!(relation.parser_generation, index.summary.parser_generation);
        assert!(relation.confidence_milli <= 1000);
    }
}

#[test]
fn references_inbound_returns_usage_sites_with_evidence() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace_with_cross_file_calls()?;
    let index = RepositoryCodeIndex::build(tmp.path(), "g1")?;
    let result = references(&index, "add", ReferencesDirection::Inbound, 100)?;

    assert!(result.summary.matched >= 2, "expected usage sites");
    assert!(!result.summary.truncated);
    let by_qualified = result
        .records
        .iter()
        .map(|record| (record.qualified_name.as_str(), record))
        .collect::<BTreeMap<_, _>>();
    let b_call = by_qualified
        .get("b::b_call")
        .ok_or("missing b call site record")?;
    let c_call = by_qualified
        .get("c::c_call")
        .ok_or("missing c call site record")?;
    assert_eq!(b_call.provenance.file_path, "app/src/b.rs");
    assert_eq!(c_call.provenance.file_path, "app/src/c.rs");

    assert!(
        result.relations.iter().any(|relation| {
            relation.kind == CodeRelationKind::Calls
                && relation.source_record_id == b_call.record_id
                && relation.target_provenance.file_path.contains("calc.rs")
        }),
        "missing evidence-backed b call relation"
    );
    assert!(
        result.relations.iter().any(|relation| {
            relation.kind == CodeRelationKind::Calls
                && relation.source_record_id == c_call.record_id
        }),
        "missing evidence-backed c call relation"
    );
    assert_relations_grounded(&index, &result);

    // Every returned record has at least one relation to the seed.
    let returned_ids = result
        .records
        .iter()
        .map(|record| record.record_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for relation in &result.relations {
        assert!(
            returned_ids.contains(relation.source_record_id.as_str()),
            "relation source is not a returned record"
        );
    }
    Ok(())
}

#[test]
fn references_outbound_returns_targets() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace_with_cross_file_calls()?;
    let index = RepositoryCodeIndex::build(tmp.path(), "g1")?;
    let result = references(&index, "b_call", ReferencesDirection::Outbound, 100)?;

    let by_qualified = result
        .records
        .iter()
        .map(|record| record.qualified_name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        by_qualified.contains("calc::add"),
        "outbound must include the called target"
    );
    assert!(
        result.relations.iter().any(|relation| {
            relation.kind == CodeRelationKind::Calls
                && relation.source_record_id.contains("b_call")
                && relation.target_record_id.contains("calc::add")
        }),
        "outbound must include the call edge"
    );
    assert_relations_grounded(&index, &result);
    Ok(())
}

#[test]
fn references_respects_direction_filters_and_bounds() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace_with_cross_file_calls()?;
    let index = RepositoryCodeIndex::build(tmp.path(), "g1")?;

    // A limit of one caps records and sets the truncation flag.
    let bounded = references(&index, "add", ReferencesDirection::Inbound, 1)?;
    assert_eq!(bounded.records.len(), 1);
    assert!(bounded.summary.truncated);
    assert!(bounded.summary.matched >= 2);
    assert_eq!(
        bounded.relations.len(),
        1,
        "relations mirror the returned record"
    );

    // An empty pattern matches every symbol; outbound still yields targets.
    let all_outbound = references(&index, "", ReferencesDirection::Outbound, 1000)?;
    assert!(all_outbound.summary.matched > 0);
    assert!(all_outbound.records.len() <= 1000);

    // Unauthorized usage sites are skipped, never errors.
    let filtered = index.references(
        CodeQuery::References {
            pattern: "add".to_string(),
            direction: ReferencesDirection::Inbound,
        },
        100,
        |symbol: &SymbolRecord| {
            Ok::<bool, Box<dyn Error>>(symbol.provenance.file_path != "app/src/b.rs")
        },
    )?;
    assert!(
        filtered
            .records
            .iter()
            .all(|record| record.provenance.file_path != "app/src/b.rs"),
        "unauthorized usage site must be skipped"
    );

    // Unknown seed: empty result, not an error.
    let missing = references(&index, "no_such_symbol", ReferencesDirection::Inbound, 100)?;
    assert_eq!(missing.summary.matched, 0);
    assert!(missing.records.is_empty());
    assert!(missing.relations.is_empty());
    Ok(())
}

#[test]
fn references_direction_parses_case_insensitively() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        "inbound".parse::<ReferencesDirection>()?,
        ReferencesDirection::Inbound
    );
    assert_eq!(
        "OUTBOUND".parse::<ReferencesDirection>()?,
        ReferencesDirection::Outbound
    );
    let error = match "sideways".parse::<ReferencesDirection>() {
        Err(error) => error,
        Ok(_) => return Err("expected direction parse failure".into()),
    };
    assert!(error.to_string().contains("inbound or outbound"));
    Ok(())
}

#[test]
fn references_rejects_other_query_kinds() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace_with_cross_file_calls()?;
    let index = RepositoryCodeIndex::build(tmp.path(), "g1")?;
    let result: Result<QueryResult, CodeIntelError> = index.references(
        CodeQuery::Symbol {
            pattern: "add".to_string(),
        },
        100,
        |_: &SymbolRecord| Ok::<bool, CodeIntelError>(true),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => return Err("expected unsupported query error".into()),
    };
    assert!(matches!(error, CodeIntelError::UnsupportedQuery { .. }));
    Ok(())
}

#[test]
fn incremental_edit_updates_references_equivalent_to_full_rebuild() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace_with_cross_file_calls()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();
    let b_path = root.join("app/src/b.rs");

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Baseline: both sibling modules are usage sites.
    let baseline = references(
        &RepositoryCodeIndex::load(&index_path)?,
        "add",
        ReferencesDirection::Inbound,
        100,
    )?;
    assert!(
        baseline
            .records
            .iter()
            .any(|record| record.qualified_name == "b::b_call")
    );
    assert!(
        baseline
            .records
            .iter()
            .any(|record| record.qualified_name == "c::c_call")
    );

    // Add a new call site in module b: the incremental rebuild must resolve
    // it exactly like a fresh full build.
    let mut source = fs::read_to_string(&b_path)?;
    source.push_str("pub fn b_extra() -> i32 { add(9, 10) }\n");
    fs::write(&b_path, source)?;
    let (incremental, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_equivalent_to_full_rebuild(&incremental, root, true)?;
    let incremental_references = incremental.references(
        CodeQuery::References {
            pattern: "add".to_string(),
            direction: ReferencesDirection::Inbound,
        },
        100,
        authorize_all(),
    )?;
    assert!(
        incremental_references
            .records
            .iter()
            .any(|record| record.qualified_name == "b::b_extra"),
        "new call site missing after incremental rebuild"
    );
    let fresh = RepositoryCodeIndex::build(root, "g1")?;
    let fresh_references = fresh.references(
        CodeQuery::References {
            pattern: "add".to_string(),
            direction: ReferencesDirection::Inbound,
        },
        100,
        authorize_all(),
    )?;
    assert_eq!(incremental_references, fresh_references);

    // Remove the call site in module b: it must disappear from references
    // after the incremental rebuild, exactly as a fresh build reports it.
    fs::write(&b_path, "pub fn b_call() -> i32 { 0 }\n")?;
    let (incremental, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_equivalent_to_full_rebuild(&incremental, root, true)?;
    let incremental_references = incremental.references(
        CodeQuery::References {
            pattern: "add".to_string(),
            direction: ReferencesDirection::Inbound,
        },
        100,
        authorize_all(),
    )?;
    assert!(
        incremental_references
            .records
            .iter()
            .all(|record| record.qualified_name != "b::b_call"),
        "removed call site still referenced"
    );
    assert!(
        incremental_references
            .records
            .iter()
            .any(|record| record.qualified_name == "c::c_call"),
        "untouched call site must survive"
    );
    let fresh = RepositoryCodeIndex::build(root, "g1")?;
    let fresh_references = fresh.references(
        CodeQuery::References {
            pattern: "add".to_string(),
            direction: ReferencesDirection::Inbound,
        },
        100,
        authorize_all(),
    )?;
    assert_eq!(incremental_references, fresh_references);
    Ok(())
}
