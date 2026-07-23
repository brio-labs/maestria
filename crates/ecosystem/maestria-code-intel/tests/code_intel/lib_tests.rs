use crate::common::*;
use maestria_code_intel::*;
use std::error::Error;

#[test]
fn build_collects_out_of_line_modules() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    write_file(
        &tmp.path().join("crate_one/src/lib.rs"),
        "pub mod external;\n",
    )?;
    write_file(
        &tmp.path().join("crate_one/src/external.rs"),
        "pub fn external_entry() {}\n",
    )?;
    let index = build_index(tmp.path(), "g1")?;
    assert!(index.symbols.iter().any(|symbol| {
        symbol.name == "external_entry" && matches!(symbol.kind, SymbolKind::Function)
    }));
    Ok(())
}

#[test]
fn external_module_cfg_context_is_preserved() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    write_file(
        &tmp.path().join("crate_one/src/lib.rs"),
        "#[cfg(test)] mod external_tests;\n#[cfg(bench)] mod external_benches;\n",
    )?;
    write_file(
        &tmp.path().join("crate_one/src/external_tests.rs"),
        "fn external_test() {}\n",
    )?;
    write_file(
        &tmp.path().join("crate_one/src/external_benches.rs"),
        "fn external_bench() {}\n",
    )?;

    let index = build_index(tmp.path(), "g1")?;
    let external_test = index
        .symbols
        .iter()
        .find(|symbol| symbol.name == "external_test")
        .ok_or("missing external test symbol")?;
    let external_bench = index
        .symbols
        .iter()
        .find(|symbol| symbol.name == "external_bench")
        .ok_or("missing external bench symbol")?;
    assert!(external_test.is_test);
    assert!(external_bench.is_bench);
    Ok(())
}

#[test]
fn query_symbol_path_and_regex_filters() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index = build_index(tmp.path(), "g1")?;

    let symbol_query = index.query(
        CodeQuery::Symbol {
            pattern: "add".to_string(),
        },
        20,
    );
    assert_eq!(symbol_query.summary.matched, 1);

    let path_query = index.query(
        CodeQuery::Path {
            pattern: "crate_one/src/lib.rs".to_string(),
        },
        20,
    );
    assert_eq!(path_query.summary.matched, index.summary.symbol_count);

    let regex_query = index.query(
        CodeQuery::Regex {
            pattern: "math::add".to_string(),
        },
        20,
    );
    assert!(regex_query.summary.matched >= 1);
    let signature_query = index.query(
        CodeQuery::Regex {
            pattern: "impl Widget".to_string(),
        },
        20,
    );
    assert_eq!(signature_query.summary.matched, 1);
    Ok(())
}

#[test]
fn provenance_and_stale_generation_identity() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index = build_index(tmp.path(), "g1")?;

    assert!(!index.summary.commit_sha.is_empty());
    assert_eq!(index.summary.parser_generation, "g1");
    assert!(!index.is_stale_generation("g1"));
    assert!(index.is_stale_generation("g2"));

    for symbol in &index.symbols {
        assert_eq!(symbol.provenance.commit_sha, index.summary.commit_sha);
        assert_eq!(
            symbol.provenance.worktree_identity,
            index.summary.worktree_identity
        );
        assert_eq!(
            symbol.provenance.repository_root,
            index.summary.repository_root
        );
        assert_eq!(
            symbol.provenance.parser_generation,
            index.summary.parser_generation
        );
    }

    Ok(())
}

#[test]
fn save_and_load_roundtrip() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace_with_routes()?;
    let path = tmp.path().join("index.json");
    let index = build_index(tmp.path(), "g2")?;
    index.save(&path)?;
    let loaded = RepositoryCodeIndex::load(&path)?;

    assert_eq!(
        loaded.summary.repository_root,
        index.summary.repository_root
    );
    assert_eq!(loaded.summary.package_count, index.summary.package_count);
    assert_eq!(loaded.summary.symbol_count, index.summary.symbol_count);
    assert_eq!(loaded.symbols, index.symbols);
    assert_eq!(
        loaded.summary.relation_summary,
        index.summary.relation_summary
    );
    assert_eq!(loaded.relations, index.relations);
    Ok(())
}

#[test]
fn markers_capture_axum_routes_and_sqlx_queries() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace_with_routes()?;
    let index = build_index(tmp.path(), "g3")?;

    let routed = match index
        .symbols
        .iter()
        .find(|symbol| symbol.name == "routed" && matches!(symbol.kind, SymbolKind::Function))
    {
        Some(symbol) => symbol,
        None => return Err("missing routed function symbol".into()),
    };

    assert!(
        routed
            .markers
            .axum_routes
            .iter()
            .any(|route| route == "get")
    );

    let query_marker = match index
        .symbols
        .iter()
        .find(|symbol| symbol.name == "query_marker")
    {
        Some(symbol) => symbol,
        None => return Err("missing query_marker function symbol".into()),
    };

    assert!(
        query_marker
            .markers
            .sqlx_queries
            .iter()
            .any(|query| query.starts_with("query"))
    );
    Ok(())
}
