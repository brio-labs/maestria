use super::common::{assert_equivalent_to_full_rebuild, build_or_update, init_git, write_file};
use maestria_code_intel::*;
use std::error::Error;
use std::fs;
use tempfile::tempdir;

/// A small Python repository: PEP 621 manifest, a `src/wishlist` package
/// with a class, functions, imports, and a call chain, plus a `tests/` dir.
pub fn make_python_repo() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let root = tempdir()?;
    write_file(
        &root.path().join("pyproject.toml"),
        r#"
[project]
name = "wishlist"
version = "0.1.0"
dependencies = ["requests>=2.0"]
"#,
    )?;
    fs::create_dir_all(root.path().join("src/wishlist"))?;
    write_file(
        &root.path().join("src/wishlist/__init__.py"),
        r#""""Wishlist package."""

from wishlist.items import Item


def create_default():
    return Item("default", 1)
"#,
    )?;
    write_file(
        &root.path().join("src/wishlist/items.py"),
        r#""""Item model."""


class Item:
    def __init__(self, name, price):
        self.name = name
        self.price = price

    def total(self, quantity):
        return self.price * quantity


def make_item(name, price=1):
    return Item(name, price)
"#,
    )?;
    write_file(
        &root.path().join("src/wishlist/orders.py"),
        r#"import wishlist.items as items
from wishlist.pricing import compute_discount


def create_order(item, quantity):
    total = item.total(quantity)
    discount = compute_discount(total)
    return total - discount
"#,
    )?;
    write_file(
        &root.path().join("src/wishlist/pricing.py"),
        r#"def compute_discount(total):
    return total * 0.1
"#,
    )?;
    fs::create_dir_all(root.path().join("tests"))?;
    write_file(
        &root.path().join("tests/test_items.py"),
        r#"from wishlist.items import Item, make_item


def test_make_item():
    item = make_item("x", 2)
    assert item.price == 2
"#,
    )?;
    init_git(root.path())?;
    Ok(root)
}

fn symbol_by_qualified<'a>(
    symbols: &'a [SymbolRecord],
    qualified: &str,
) -> Result<&'a SymbolRecord, Box<dyn Error>> {
    symbols
        .iter()
        .find(|symbol| symbol.qualified_name == qualified)
        .ok_or_else(|| format!("missing symbol {qualified}").into())
}

#[test]
fn python_repository_indexes_real_symbols() -> Result<(), Box<dyn Error>> {
    let root = make_python_repo()?;
    let index = RepositoryCodeIndex::build(root.path(), "g1")?;

    assert_eq!(index.summary.package_count, 1);
    assert_eq!(index.summary.packages, vec!["wishlist".to_string()]);
    let wishlist = index
        .packages
        .iter()
        .find(|package| package.name == "wishlist")
        .ok_or("missing wishlist package")?;
    assert_eq!(wishlist.manifest_path, "pyproject.toml");
    assert_eq!(wishlist.version, "0.1.0");
    assert_eq!(wishlist.dependencies.len(), 1);
    assert_eq!(wishlist.dependencies[0].name, "requests");
    assert_eq!(wishlist.dependencies[0].version_req, ">=2.0");
    // The distribution has a package target, a tests target, and no
    // root-level module targets.
    let target_kinds: Vec<&str> = wishlist
        .targets
        .iter()
        .map(|target| target.kind[0].as_str())
        .collect();
    assert!(target_kinds.contains(&"py-module"));
    assert!(target_kinds.contains(&"py-test"));

    // Module, class, method, function, and import symbols with qualified
    // dotted names and the shared record_id format.
    assert_eq!(index.summary.symbol_count, 18);
    let module = symbol_by_qualified(&index.symbols, "wishlist.items")?;
    assert_eq!(module.kind, SymbolKind::Module);
    assert_eq!(
        module.record_id,
        "src/wishlist/items.py:module:wishlist.items:1-15"
    );
    let class = symbol_by_qualified(&index.symbols, "wishlist.items.Item")?;
    assert_eq!(class.kind, SymbolKind::Class);
    assert_eq!(class.signature.as_deref(), Some("class Item"));
    assert!(class.is_public_api);
    assert_eq!(
        class.record_id,
        "src/wishlist/items.py:class:wishlist.items.Item:4-10"
    );
    let method = symbol_by_qualified(&index.symbols, "wishlist.items.Item.total")?;
    assert_eq!(method.kind, SymbolKind::Method);
    assert_eq!(
        method.record_id,
        "src/wishlist/items.py:method:wishlist.items.Item.total:9-10"
    );
    let function = symbol_by_qualified(&index.symbols, "wishlist.orders.create_order")?;
    assert_eq!(function.kind, SymbolKind::Function);
    assert_eq!(
        function.record_id,
        "src/wishlist/orders.py:function:wishlist.orders.create_order:5-8"
    );

    // Test files are flagged is_test; the module path covers the tests dir.
    let test = symbol_by_qualified(&index.symbols, "tests.test_items.test_make_item")?;
    assert!(test.is_test);
    assert_eq!(
        test.record_id,
        "tests/test_items.py:function:tests.test_items.test_make_item:4-6"
    );

    // Imports become Import symbols and relations.
    let import = symbol_by_qualified(&index.symbols, "wishlist.orders.items")?;
    assert_eq!(import.kind, SymbolKind::Import);
    assert_eq!(import.imports, vec!["wishlist.items".to_string()]);

    let import_kinds: Vec<CodeRelationKind> = index
        .relations
        .iter()
        .filter(|relation| relation.kind == CodeRelationKind::Imports)
        .map(|relation| relation.kind)
        .collect();
    assert_eq!(import_kinds.len(), 5, "expected 5 import relations");
    let call_kinds: Vec<CodeRelationKind> = index
        .relations
        .iter()
        .filter(|relation| relation.kind == CodeRelationKind::Calls)
        .map(|relation| relation.kind)
        .collect();
    assert_eq!(call_kinds.len(), 5, "expected 5 call relations");
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id.contains("create_order")
            && relation.target_record_id.contains("Item.total")
    }));
    index.validate_provenance()?;
    Ok(())
}

#[test]
fn python_symbols_are_searchable() -> Result<(), Box<dyn Error>> {
    let root = make_python_repo()?;
    let index = RepositoryCodeIndex::build(root.path(), "g1")?;

    let result = index.query(
        CodeQuery::Symbol {
            pattern: "create_order".to_string(),
        },
        10,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(result.summary.matched, 1);
    assert_eq!(
        result.records[0].qualified_name,
        "wishlist.orders.create_order"
    );

    let result = index.query(
        CodeQuery::Path {
            pattern: "src/wishlist".to_string(),
        },
        100,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert!(result.summary.matched >= 14);

    let result = index.query(
        CodeQuery::Regex {
            pattern: "Item\\.total".to_string(),
        },
        10,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(result.summary.matched, 1);
    assert_eq!(
        result.records[0].qualified_name,
        "wishlist.items.Item.total"
    );
    Ok(())
}

#[test]
fn python_context_traverses_import_and_call_relations() -> Result<(), Box<dyn Error>> {
    let root = make_python_repo()?;
    let index = RepositoryCodeIndex::build(root.path(), "g1")?;

    let result = index.context(
        RepositoryContextQuery {
            query: CodeQuery::Symbol {
                pattern: "create_order".to_string(),
            },
            direction: ContextDirection::Outgoing,
            relation_kinds: None,
            max_depth: 2,
            max_nodes: 32,
        },
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    // The seed plus every node reached through imports/calls.
    assert_eq!(result.summary.seed_query.matched, 1);
    assert!(
        result.summary.matched_nodes >= 3,
        "expected seed plus reached nodes"
    );
    let reached: Vec<&str> = result
        .nodes
        .iter()
        .map(|node| node.record.qualified_name.as_str())
        .collect();
    assert!(reached.contains(&"wishlist.items.Item.total"));
    assert!(reached.contains(&"wishlist.pricing.compute_discount"));
    assert!(reached.contains(&"wishlist.orders.create_order"));
    // The call edges also carry the resolved import/call chain: the
    // traversal must see at least one outgoing edge from the seed.
    assert!(
        result
            .edges
            .iter()
            .any(|edge| edge.relation.source_record_id.contains("create_order"))
    );
    Ok(())
}

#[test]
fn python_incremental_edit_equals_full_rebuild() -> Result<(), Box<dyn Error>> {
    let root = make_python_repo()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");

    let (_, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Append a function to a tracked file.
    let items = root.path().join("src/wishlist/items.py");
    let mut source = fs::read_to_string(&items)?;
    source
        .push_str("\n\ndef discounted_price(price):\n    return price - compute_discount(price)\n");
    fs::write(&items, source)?;

    let (incremental, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        incremental
            .symbols
            .iter()
            .any(|symbol| symbol.qualified_name == "wishlist.items.discounted_price")
    );
    assert_equivalent_to_full_rebuild(&incremental, root.path(), true)?;

    // Unchanged repository is a no-op.
    let (_, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Noop);
    Ok(())
}

#[test]
fn python_manifest_edit_changes_identity_and_forces_full() -> Result<(), Box<dyn Error>> {
    let root = make_python_repo()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");

    let (index, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    let before = index.summary.worktree_identity.clone();

    // Editing pyproject.toml changes the worktree identity and forces a full
    // rebuild (discovery input changed).
    let manifest = root.path().join("pyproject.toml");
    let mut source = fs::read_to_string(&manifest)?;
    source.push_str("\n[project.optional-dependencies]\ndev = [\"pytest\"]\n");
    fs::write(&manifest, source)?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert_ne!(before, index.summary.worktree_identity);
    Ok(())
}

#[test]
fn python_new_file_is_dirty_and_forces_full() -> Result<(), Box<dyn Error>> {
    let root = make_python_repo()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");

    let (_, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // A new module inside an existing package is extractable by a full
    // rebuild without any manifest change, so only Full can pick it up.
    write_file(
        &root.path().join("src/wishlist/extra.py"),
        "def extra_helper():\n    return 1\n",
    )?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.qualified_name == "wishlist.extra.extra_helper")
    );
    Ok(())
}

#[test]
fn python_new_package_forces_full() -> Result<(), Box<dyn Error>> {
    let root = make_python_repo()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");

    let (_, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // A new top-level package under src/ is an auto-discovery target.
    fs::create_dir_all(root.path().join("src/newpkg"))?;
    write_file(
        &root.path().join("src/newpkg/__init__.py"),
        "def new_pkg_fn():\n    return 2\n",
    )?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.qualified_name == "newpkg.new_pkg_fn")
    );
    Ok(())
}

#[test]
fn python_new_test_file_forces_full() -> Result<(), Box<dyn Error>> {
    let root = make_python_repo()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");

    let (_, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    write_file(
        &root.path().join("tests/test_extra.py"),
        "def test_extra():\n    assert True\n",
    )?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.qualified_name == "tests.test_extra.test_extra" && symbol.is_test)
    );
    Ok(())
}
