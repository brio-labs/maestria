use maestria_code_intel::*;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn Error>> {
    fs::write(path, contents)?;
    Ok(())
}

fn init_git(repo_root: &Path) -> Result<(), Box<dyn Error>> {
    run_git(repo_root, &["init", "--initial-branch", "main"], "git init")?;
    run_git(
        repo_root,
        &["config", "user.email", "ci@example.com"],
        "git config user.email",
    )?;
    run_git(
        repo_root,
        &["config", "user.name", "CI"],
        "git config user.name",
    )?;
    run_git(repo_root, &["add", "."], "git add")?;
    run_git(repo_root, &["commit", "-m", "fixture init"], "git commit")
}

fn run_git(repo_root: &Path, args: &[&str], operation: &str) -> Result<(), Box<dyn Error>> {
    let status = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .status()?;
    if !status.success() {
        return Err(format!(
            "{operation} failed in {}: exit {status}",
            repo_root.display()
        )
        .into());
    }
    Ok(())
}

fn build_index(
    root: &Path,
    parser_generation: &str,
) -> Result<RepositoryCodeIndex, Box<dyn Error>> {
    Ok(RepositoryCodeIndex::build(root, parser_generation)?)
}

fn make_workspace() -> Result<tempfile::TempDir, Box<dyn Error>> {
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
        r#"
pub mod math {
    /// Adds two numbers.
    pub fn add(a: i32, b: i32) -> i32 { a + b }
}

pub struct Widget {
    pub value: i32,
}

impl Widget {
    pub fn name(&self) -> i32 { self.value }
}
"#,
    )?;
    init_git(root.path())?;
    Ok(root)
}

fn make_workspace_with_routes() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let root = tempdir()?;
    write_file(
        &root.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"api\"]\n",
    )?;
    fs::create_dir_all(root.path().join("api/src"))?;
    write_file(
        &root.path().join("api/Cargo.toml"),
        r#"
[package]
name = "api"
version = "0.1.0"
edition = "2024"
[dependencies]
axum = { version = "0.7", optional = true }
sqlx = { version = "0.7", optional = true }
"#,
    )?;
    write_file(
        &root.path().join("api/src/lib.rs"),
        r#"
use axum::routing::get;

pub fn route_index() -> usize {
    1
}

#[get("/")]
fn routed() {}

fn query_marker() {
    let _ = sqlx::query("SELECT 1");
}
"#,
    )?;
    init_git(root.path())?;
    Ok(root)
}

fn relation_lib_source() -> String {
    [relation_lib_head(), relation_lib_tail()].concat()
}

fn relation_lib_head() -> &'static str {
    r#"
pub mod helpers;

use crate::Notifier;
use crate::math::add;
use crate::math::add as plus;
use crate::math as m;

pub mod math {
    pub fn add(a: i32, b: i32) -> i32 {
        a + b
    }
}

pub trait Notifier {
    fn notify(&self, value: i32) -> i32 {
        value
    }
}

pub mod nested {
    pub fn helper() -> i32 {
        1
    }

    pub fn caller() -> i32 {
        helper()
    }

    pub fn self_caller() -> i32 {
        self::helper()
    }
}

pub struct Widget {
    pub value: i32,
}

impl Widget {
    pub fn helper(&self) -> i32 {
        self.value
    }

    pub fn calls_self(&self) -> i32 {
        self.helper()
    }
}

impl Notifier for Widget {
    fn notify(&self, value: i32) -> i32 {
        value
    }
}

pub mod method_scope {
    pub fn helper() -> i32 {
        1
    }

    pub struct Widget;

    impl Widget {
        pub fn helper(&self) -> i32 {
            2
        }

        pub fn caller(&self) -> i32 {
            helper()
        }

        pub fn self_type_caller(&self) -> i32 {
            Self::helper(self)
        }
    }
}

"#
}

fn relation_lib_tail() -> &'static str {
    r#"pub fn imported_add_usage() -> i32 {
    math::add(3, 4)
}

pub fn module_alias_usage() -> i32 {
    m::add(3, 4)
}

pub fn alias_usage() -> i32 {
    plus(5, 6)
}

pub fn helpers_call() -> i32 {
    helpers::from_helpers()
}

pub fn target() -> i32 {
    1
}

pub fn self_root_call() -> i32 {
    self::target()
}

pub fn shadowed_call() -> i32 {
    let target = || 2;
    target()
}

pub fn initializer_call() -> i32 {
    let target = target();
    target
}

pub fn scope_leak_call() -> i32 {
    {
        let target = || 2;
        target()
    }
    target()
}

pub fn if_let_scope_call() -> i32 {
    if let Some(target) = Some(2) {
        target
    }
    target()
}

pub fn while_let_scope_call() -> i32 {
    while let Some(target) = Some(2) {
        let _ = target;
        break;
    }
    target()
}

pub fn unresolved() -> i32 {
    unknown_symbol();
    0
}

#[cfg(test)]
mod tests {
    #[test]
    fn relations_are_seen() {
        let _ = super::math::add(3, 4);
        let widget = super::Widget { value: 7 };
        widget.calls_self();
    }
}
"#
}

fn make_workspace_with_relations() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let root = tempdir()?;
    write_file(
        &root.path().join("Cargo.toml"),
        r#"
[workspace]
members = ["relations"]

[workspace.package]
edition = "2024"
"#,
    )?;
    fs::create_dir_all(root.path().join("relations/src"))?;
    write_file(
        &root.path().join("relations/Cargo.toml"),
        r#"
[package]
name = "relations"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        &root.path().join("relations/src/lib.rs"),
        &relation_lib_source(),
    )?;
    write_file(
        &root.path().join("relations/src/helpers.rs"),
        "pub fn from_helpers() -> i32 { 6 }\n",
    )?;
    init_git(root.path())?;
    Ok(root)
}

fn symbol_id<'a>(
    symbols: &'a [SymbolRecord],
    qualified_name: &str,
) -> Result<&'a str, Box<dyn Error>> {
    symbols
        .iter()
        .find(|symbol| symbol.qualified_name == qualified_name)
        .map(|symbol| symbol.record_id.as_str())
        .ok_or_else(|| format!("missing symbol {qualified_name}").into())
}

fn relation_sort_key(
    relation: &CodeRelationRecord,
) -> (u8, &str, &str, &str, usize, usize, &str, usize, u16) {
    let kind = match relation.kind {
        CodeRelationKind::Defines => 0,
        CodeRelationKind::Imports => 1,
        CodeRelationKind::Calls => 2,
        CodeRelationKind::Implements => 3,
        CodeRelationKind::Tests => 4,
    };
    (
        kind,
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
