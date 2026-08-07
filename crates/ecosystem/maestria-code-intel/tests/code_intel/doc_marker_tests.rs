//! Doc-comment and code-marker search: extraction, query semantics, and
//! incremental equivalence (issue #416).

use crate::common::{
    assert_equivalent_to_full_rebuild, build_or_update, make_workspace, write_file,
};
use maestria_code_intel::*;
use std::error::Error;
use std::fs;
use tempfile::tempdir;

/// A workspace whose lib.rs carries `///` docs, a `// todo` comment, a
/// `// HACK` comment, a file-level `//!` doc, and an `unsafe` block.
fn make_documented_workspace() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let root = tempdir()?;
    write_file(
        &root.path().join("Cargo.toml"),
        r#"
[workspace]
members = ["crate_one"]

[workspace.package]
edition = "2024"
"#,
    )?;
    fs::create_dir_all(root.path().join("crate_one/src"))?;
    write_file(
        &root.path().join("crate_one/Cargo.toml"),
        r#"
[package]
name = "crate_one"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        &root.path().join("crate_one/src/lib.rs"),
        r#"//! File-level module docs.

/// Adds two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    // todo: handle overflow
    let _ = unsafe { a + b };
    a + b
}

// HACK: orphan comment outside every symbol
"#,
    )?;
    crate::common::init_git(root.path())?;
    Ok(root)
}

#[test]
fn doc_query_returns_documented_symbols() -> Result<(), Box<dyn Error>> {
    let root = make_documented_workspace()?;
    let index = crate::common::build_index(root.path(), "g1")?;

    let result = index.query(
        CodeQuery::Doc {
            pattern: "Adds two numbers".to_string(),
        },
        20,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(result.summary.matched, 1);
    let add = result
        .records
        .iter()
        .find(|record| record.qualified_name == "add")
        .ok_or("doc query must return the `add` function")?;
    assert_eq!(add.doc_comment.as_deref(), Some("Adds two numbers."));

    // File-level `//!` docs attach to the file's root module symbol.
    let module_docs = index.query(
        CodeQuery::Doc {
            pattern: "File-level module docs".to_string(),
        },
        20,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(module_docs.summary.matched, 1);
    let module = &module_docs.records[0];
    assert_eq!(module.kind, SymbolKind::Module);
    assert_eq!(module.qualified_name, "crate");

    // A pattern absent from every doc comment matches nothing.
    let missing = index.query(
        CodeQuery::Doc {
            pattern: "no such documentation".to_string(),
        },
        20,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(missing.summary.matched, 0);
    Ok(())
}

#[test]
fn marker_query_is_deterministic_and_range_accurate() -> Result<(), Box<dyn Error>> {
    let root = make_documented_workspace()?;
    let index = crate::common::build_index(root.path(), "g1")?;

    // The todo comment is inside `add`'s range and attaches to it.
    let todo = index.query(
        CodeQuery::Markers {
            marker_kind: MarkerQueryKind::Todo,
        },
        20,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(todo.summary.matched, 1);
    let add = todo
        .records
        .iter()
        .find(|record| record.qualified_name == "add")
        .ok_or("todo marker must attach to `add`")?;
    assert_eq!(add.markers.code_markers.len(), 1);
    let marker = &add.markers.code_markers[0];
    assert_eq!(marker.kind(), CodeMarkerKind::Todo);
    assert!(marker.start_line() <= marker.end_line());
    assert!(marker.start_line() >= 1);
    let range = &add.provenance.source_range;
    assert!(
        range.start_line() <= marker.start_line() && marker.end_line() <= range.end_line(),
        "marker range must sit inside the symbol range"
    );

    // The orphan HACK comment attaches to the file's root module symbol.
    let hack = index.query(
        CodeQuery::Markers {
            marker_kind: MarkerQueryKind::Hack,
        },
        20,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(hack.summary.matched, 1);
    let module = hack
        .records
        .iter()
        .find(|record| record.qualified_name == "crate")
        .ok_or("orphan HACK marker must attach to the file module")?;
    assert_eq!(module.markers.code_markers.len(), 1);
    assert_eq!(module.markers.code_markers[0].kind(), CodeMarkerKind::Hack);

    // The `unsafe` block is an UnsafeBlock symbol with a precise range.
    let unsafe_hits = index.query(
        CodeQuery::Markers {
            marker_kind: MarkerQueryKind::Unsafe,
        },
        20,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    let block = unsafe_hits
        .records
        .iter()
        .find(|record| record.kind == SymbolKind::UnsafeBlock)
        .ok_or("unsafe marker query must surface the UnsafeBlock symbol")?;
    assert!(block.is_unsafe);
    let block_range = &block.provenance.source_range;
    assert!(
        range.start_line() <= block_range.start_line()
            && block_range.end_line() <= range.end_line(),
        "unsafe block range must sit inside the enclosing function"
    );
    Ok(())
}

#[test]
fn incremental_doc_and_marker_edit_equals_full_rebuild() -> Result<(), Box<dyn Error>> {
    let root = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");

    let (_, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Edit the tracked lib.rs: add a `///` doc, a `// todo` comment, and an
    // `unsafe` block, then rebuild incrementally.
    let lib_path = root.path().join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str(
        "\n/// Documented by the edit.\npub fn edited() -> i32 {\n    // todo: revisit\n    let _ = unsafe { 1 };\n    1\n}\n",
    );
    fs::write(&lib_path, source)?;

    let (incremental, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_equivalent_to_full_rebuild(&incremental, root.path(), true)?;

    // The incremental index carries the doc text and markers.
    let edited = incremental
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == "edited")
        .ok_or("missing edited function")?;
    assert_eq!(
        edited.doc_comment.as_deref(),
        Some("Documented by the edit.")
    );
    assert_eq!(edited.markers.code_markers.len(), 1);
    assert_eq!(edited.markers.code_markers[0].kind(), CodeMarkerKind::Todo);
    assert!(
        incremental
            .symbols
            .iter()
            .any(|symbol| { symbol.kind == SymbolKind::UnsafeBlock && symbol.is_unsafe })
    );
    Ok(())
}

#[test]
fn persisted_index_round_trips_doc_and_markers() -> Result<(), Box<dyn Error>> {
    let root = make_documented_workspace()?;
    let path = root.path().join("index.json");
    let index = crate::common::build_index(root.path(), "g1")?;
    index.save(&path)?;
    let loaded = RepositoryCodeIndex::load(&path)?;
    assert_eq!(loaded.symbols, index.symbols);
    Ok(())
}
