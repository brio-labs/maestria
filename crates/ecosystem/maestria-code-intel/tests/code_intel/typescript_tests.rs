use super::common::{assert_equivalent_to_full_rebuild, build_or_update, init_git, write_file};
use maestria_code_intel::*;
use std::error::Error;
use std::fs;
use tempfile::tempdir;

/// A small TypeScript repository: a `package.json` with a `src/` tree
/// (components, models, utils), a `tests/` dir, an import chain across
/// files, an arrow JSX component, a class, an interface, a type alias, and a
/// const binding.
pub fn make_web_repo() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let root = tempdir()?;
    write_file(
        &root.path().join("package.json"),
        r#"{
  "name": "ui-kit",
  "version": "0.1.0",
  "main": "src/index.ts"
}
"#,
    )?;
    write_file(
        &root.path().join("src/index.ts"),
        r#"import { Button } from "./components/Button";
import { Card } from "./components/Card";
import { Item } from "./models/Item";

export { Button, Card };

export function createDefaultItem(): Item {
  return new Item("default", 1);
}

export const catalogSize = 3;
"#,
    )?;
    write_file(
        &root.path().join("src/components/Button.tsx"),
        r#"export interface ButtonProps {
  label: string;
  disabled?: boolean;
}

export function Button({ label, disabled }: ButtonProps) {
  return <button disabled={disabled}>{label}</button>;
}
"#,
    )?;
    write_file(
        &root.path().join("src/components/Card.tsx"),
        r#"export const Card = ({ title }: { title: string }) => (
  <div className="card">
    <h2>{title}</h2>
  </div>
);
"#,
    )?;
    write_file(
        &root.path().join("src/models/Item.ts"),
        r#"export class Item {
  name: string;
  price: number;

  constructor(name: string, price: number) {
    this.name = name;
    this.price = price;
  }

  total(quantity: number): number {
    return this.price * quantity;
  }
}

export function makeItem(name: string, price = 1) {
  return new Item(name, price);
}
"#,
    )?;
    write_file(
        &root.path().join("src/utils/format.ts"),
        r#"function pad(value: number): string {
  return String(value).padStart(2, "0");
}

export function formatPrice(cents: number): string {
  const value = pad(cents / 100);
  return `$${value}`;
}
"#,
    )?;
    write_file(
        &root.path().join("src/types.ts"),
        r#"export type Size = "small" | "medium" | "large";

export const sizes: Size[] = ["small", "medium", "large"];
"#,
    )?;
    write_file(
        &root.path().join("tests/button.test.ts"),
        r#"import { Button } from "../src/components/Button";

export function renderButton(): string {
  const label = "Go";
  return Button({ label }).label;
}
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
fn web_repository_indexes_real_symbols() -> Result<(), Box<dyn Error>> {
    let root = make_web_repo()?;
    let index = RepositoryCodeIndex::build(root.path(), "g1")?;

    assert_eq!(index.summary.package_count, 1);
    assert_eq!(index.summary.packages, vec!["ui-kit".to_string()]);
    assert_eq!(index.summary.target_count, 3);
    assert_eq!(index.summary.symbol_count, 25);
    assert_eq!(index.summary.file_count, 7);
    let package = index
        .packages
        .iter()
        .find(|package| package.name == "ui-kit")
        .ok_or("missing ui-kit package")?;
    assert_eq!(package.manifest_path, "package.json");
    assert_eq!(package.version, "0.1.0");
    let target_kinds: Vec<&str> = package
        .targets
        .iter()
        .map(|target| target.kind[0].as_str())
        .collect();
    assert!(target_kinds.contains(&"web-src"));
    assert!(target_kinds.contains(&"web-test"));

    // Module, component function, class, method, interface/type, const, and
    // import symbols with the shared record_id format.
    let module = symbol_by_qualified(&index.symbols, "src/components/Button")?;
    assert_eq!(module.kind, SymbolKind::Module);
    assert_eq!(
        module.record_id,
        "src/components/Button.tsx:module:src/components/Button:1-9"
    );
    let component = symbol_by_qualified(&index.symbols, "src/components/Button::Button")?;
    assert_eq!(component.kind, SymbolKind::Function);
    assert!(component.is_public_api);
    assert_eq!(
        component.record_id,
        "src/components/Button.tsx:function:src/components/Button::Button:6-8"
    );
    let interface_symbol =
        symbol_by_qualified(&index.symbols, "src/components/Button::ButtonProps")?;
    assert_eq!(interface_symbol.kind, SymbolKind::TypeAlias);
    assert!(interface_symbol.is_public_api);
    let class = symbol_by_qualified(&index.symbols, "src/models/Item::Item")?;
    assert_eq!(class.kind, SymbolKind::Class);
    let method = symbol_by_qualified(&index.symbols, "src/models/Item::Item::total")?;
    assert_eq!(method.kind, SymbolKind::Method);
    let const_symbol = symbol_by_qualified(&index.symbols, "src/types::sizes")?;
    assert_eq!(const_symbol.kind, SymbolKind::Const);
    assert!(const_symbol.is_public_api);

    // The arrow component is a Function; a non-exported helper is not public.
    let arrow_component = symbol_by_qualified(&index.symbols, "src/components/Card::Card")?;
    assert_eq!(arrow_component.kind, SymbolKind::Function);
    assert!(arrow_component.is_public_api);
    let helper = symbol_by_qualified(&index.symbols, "src/utils/format::pad")?;
    assert_eq!(helper.kind, SymbolKind::Function);
    assert!(
        !helper.is_public_api,
        "non-exported helper must not be public"
    );

    // Test files are flagged is_test with the module path covering the dir.
    let test = symbol_by_qualified(&index.symbols, "tests/button.test::renderButton")?;
    assert!(test.is_test);
    assert_eq!(
        test.record_id,
        "tests/button.test.ts:function:tests/button.test::renderButton:3-6"
    );

    // Imports become Import symbols.
    let import = symbol_by_qualified(&index.symbols, "src/index::Button")?;
    assert_eq!(import.kind, SymbolKind::Import);
    assert_eq!(import.imports, vec!["./components/Button".to_string()]);
    assert!(!import.is_public_api);
    index.validate_provenance()?;
    Ok(())
}

#[test]
fn web_relations_resolve_imports_and_calls() -> Result<(), Box<dyn Error>> {
    let root = make_web_repo()?;
    let index = RepositoryCodeIndex::build(root.path(), "g1")?;

    let import_kinds: Vec<CodeRelationKind> = index
        .relations
        .iter()
        .filter(|relation| relation.kind == CodeRelationKind::Imports)
        .map(|relation| relation.kind)
        .collect();
    assert_eq!(import_kinds.len(), 8, "expected 8 import relations");
    let call_kinds: Vec<CodeRelationKind> = index
        .relations
        .iter()
        .filter(|relation| relation.kind == CodeRelationKind::Calls)
        .map(|relation| relation.kind)
        .collect();
    assert_eq!(call_kinds.len(), 4, "expected 4 call relations");
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Calls
            && relation.source_record_id.contains("createDefaultItem")
            && relation
                .target_record_id
                .contains("src/models/Item.ts:class:")
    }));
    assert!(index.relations.iter().any(|relation| {
        relation.kind == CodeRelationKind::Imports
            && relation.source_record_id.contains("tests/button.test.ts")
            && relation
                .target_record_id
                .contains("src/components/Button.tsx:function:")
    }));
    Ok(())
}

#[test]
fn web_symbols_are_searchable() -> Result<(), Box<dyn Error>> {
    let root = make_web_repo()?;
    let index = RepositoryCodeIndex::build(root.path(), "g1")?;

    let result = index.query(
        CodeQuery::Symbol {
            pattern: "createDefaultItem".to_string(),
        },
        10,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(result.summary.matched, 1);
    assert_eq!(
        result.records[0].qualified_name,
        "src/index::createDefaultItem"
    );

    // Component and function names are findable.
    let result = index.query(
        CodeQuery::Symbol {
            pattern: "Card".to_string(),
        },
        10,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert!(
        result
            .records
            .iter()
            .any(|record| record.qualified_name == "src/components/Card::Card"),
        "arrow component must be searchable by name"
    );

    let result = index.query(
        CodeQuery::Path {
            pattern: "src/components".to_string(),
        },
        100,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(result.summary.matched, 5, "component files carry 5 symbols");

    let result = index.query(
        CodeQuery::Regex {
            pattern: "Item::total".to_string(),
        },
        10,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(result.summary.matched, 1);
    assert_eq!(
        result.records[0].qualified_name,
        "src/models/Item::Item::total"
    );
    Ok(())
}

#[test]
fn web_context_traverses_import_and_call_relations() -> Result<(), Box<dyn Error>> {
    let root = make_web_repo()?;
    let index = RepositoryCodeIndex::build(root.path(), "g1")?;

    let result = index.context(
        RepositoryContextQuery {
            query: CodeQuery::Regex {
                pattern: "^src/components/Button::Button$".to_string(),
            },
            direction: ContextDirection::Both,
            relation_kinds: None,
            max_depth: 2,
            max_nodes: 32,
        },
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(result.summary.seed_query.matched, 1);
    let reached: Vec<&str> = result
        .nodes
        .iter()
        .map(|node| node.record.qualified_name.as_str())
        .collect();
    // The component seed reaches its importing module symbol, the import
    // binding in index.ts, and the test that calls it.
    assert!(reached.contains(&"src/components/Button"), "module symbol");
    assert!(reached.contains(&"src/index::Button"), "import binding");
    assert!(
        reached.contains(&"tests/button.test::renderButton"),
        "calling test function"
    );
    assert!(
        result
            .edges
            .iter()
            .any(|edge| edge.relation.kind == CodeRelationKind::Calls),
        "expected at least one call edge"
    );
    Ok(())
}

#[test]
fn web_incremental_edit_equals_full_rebuild() -> Result<(), Box<dyn Error>> {
    let root = make_web_repo()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");

    let (_, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Append a function to a tracked file.
    let items = root.path().join("src/models/Item.ts");
    let mut source = fs::read_to_string(&items)?;
    source.push_str("\nexport function discounted(price: number) {\n  return price - 1;\n}\n");
    fs::write(&items, source)?;

    let (incremental, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        incremental
            .symbols
            .iter()
            .any(|symbol| symbol.qualified_name == "src/models/Item::discounted")
    );
    assert_equivalent_to_full_rebuild(&incremental, root.path(), true)?;

    // Unchanged repository is a no-op.
    let (_, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Noop);
    Ok(())
}

#[test]
fn web_manifest_edit_changes_identity_and_forces_full() -> Result<(), Box<dyn Error>> {
    let root = make_web_repo()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");

    let (index, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    let before = index.summary.worktree_identity.clone();

    // Editing package.json changes the worktree identity and forces a full
    // rebuild (discovery input changed).
    let manifest = root.path().join("package.json");
    let mut source = fs::read_to_string(&manifest)?;
    source = source.replace("\"0.1.0\"", "\"0.1.1\"");
    fs::write(&manifest, source)?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert_ne!(before, index.summary.worktree_identity);
    Ok(())
}

#[test]
fn web_new_src_file_is_auto_target_and_forces_full() -> Result<(), Box<dyn Error>> {
    let root = make_web_repo()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");

    let (_, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // A new component under src/ is extractable by a full rebuild without
    // any manifest change, so only Full can pick it up.
    write_file(
        &root.path().join("src/components/Badge.tsx"),
        "export function Badge({ text }: { text: string }) {\n  return <span>{text}</span>;\n}\n",
    )?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root.path())?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.qualified_name == "src/components/Badge::Badge")
    );
    Ok(())
}

#[test]
fn web_repo_without_manifest_is_empty() -> Result<(), Box<dyn Error>> {
    let root = tempdir()?;
    write_file(
        &root.path().join("app.js"),
        "export function standalone() {\n  return 1;\n}\n",
    )?;
    init_git(root.path())?;

    // A `.js`-only repository without any package.json is not a web repo:
    // detection is conservative and the index is empty (mirrors Python).
    let index = RepositoryCodeIndex::build(root.path(), "g1")?;
    assert_eq!(index.summary.package_count, 0);
    assert_eq!(index.summary.symbol_count, 0);
    let result = index.query(
        CodeQuery::Symbol {
            pattern: "standalone".to_string(),
        },
        10,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(result.summary.matched, 0);
    Ok(())
}

#[test]
fn web_repo_indexes_stray_js_when_package_exists() -> Result<(), Box<dyn Error>> {
    // A package.json without src/ or entry points falls back to its own
    // directory, so a root-level `.js` module is still indexed.
    let root = tempdir()?;
    write_file(&root.path().join("package.json"), "{\"name\": \"tiny\"}\n")?;
    write_file(
        &root.path().join("index.js"),
        "export function tinyMain() {\n  return 42;\n}\n",
    )?;
    init_git(root.path())?;

    let index = RepositoryCodeIndex::build(root.path(), "g1")?;
    assert_eq!(index.summary.package_count, 1);
    assert_eq!(index.summary.packages, vec!["tiny".to_string()]);
    let module = symbol_by_qualified(&index.symbols, "index")?;
    assert_eq!(module.kind, SymbolKind::Module);
    let function = symbol_by_qualified(&index.symbols, "index::tinyMain")?;
    assert_eq!(function.kind, SymbolKind::Function);
    index.validate_provenance()?;
    Ok(())
}
