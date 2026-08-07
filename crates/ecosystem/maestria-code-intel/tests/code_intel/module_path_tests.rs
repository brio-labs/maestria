use super::common::{
    assert_equivalent_to_full_rebuild, build_index, build_or_update, init_git, write_file,
};
use maestria_code_intel::*;
use std::error::Error;
use std::fs;
use tempfile::tempdir;

/// Workspace crate `app` whose `lib.rs` declares a `#[path]`-attributed module
/// (`core/crate_root.rs`) plus a plain `mod plain;`, where the `#[path]` module
/// nests a file-backed `mod ser;` inside an inline `mod lib { }` block —
/// mirroring serde's `crate_root` layout without the macro indirection.
fn make_app_with_path_and_nested_mods() -> Result<tempfile::TempDir, Box<dyn Error>> {
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
    fs::create_dir_all(root.path().join("app/src/core"))?;
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
        r#"
#[path = "core/crate_root.rs"]
mod crate_root;

mod plain;
"#,
    )?;
    write_file(
        &root.path().join("app/src/core/crate_root.rs"),
        "mod lib {\n    pub mod ser;\n}\n",
    )?;
    write_file(
        &root.path().join("app/src/core/ser/mod.rs"),
        "pub trait Serializer {\n    fn serialize(&self) -> u8;\n}\n",
    )?;
    write_file(
        &root.path().join("app/src/plain.rs"),
        "pub fn plain_fn() -> u8 { 1 }\n",
    )?;
    init_git(root.path())?;
    Ok(root)
}

#[test]
fn path_attribute_and_nested_inline_modules_are_followed() -> Result<(), Box<dyn Error>> {
    let tmp = make_app_with_path_and_nested_mods()?;
    let index = build_index(tmp.path(), "g1")?;

    // The Serializer trait is reachable only through the #[path] module whose
    // inline `mod lib { }` block declares the file-backed `mod ser;`.
    let serializer = index
        .symbols
        .iter()
        .find(|symbol| symbol.name == "Serializer" && symbol.kind == SymbolKind::Trait)
        .ok_or("missing Serializer trait symbol")?;
    assert_eq!(
        serializer.qualified_name,
        "crate_root::lib::ser::Serializer"
    );
    assert_eq!(serializer.provenance.file_path, "app/src/core/ser/mod.rs");
    assert!(serializer.is_public_api);

    // The nested-inline module file carries the full module stack and the
    // file that declared it.
    let ser_context = index
        .file_contexts
        .get("app/src/core/ser/mod.rs")
        .ok_or("missing ser/mod.rs file context")?;
    assert_eq!(ser_context.stack, vec!["crate_root", "lib", "ser"]);
    assert_eq!(
        ser_context.parent.as_deref(),
        Some("app/src/core/crate_root.rs")
    );

    // The #[path]-declared module file resolves against the declaring file's
    // directory and keeps the plain file-backed stack.
    let crate_root_context = index
        .file_contexts
        .get("app/src/core/crate_root.rs")
        .ok_or("missing crate_root.rs file context")?;
    assert_eq!(crate_root_context.stack, vec!["crate_root"]);
    assert_eq!(crate_root_context.parent.as_deref(), Some("app/src/lib.rs"));

    // The plain `mod plain;` sibling is still followed from the same base.
    let plain_context = index
        .file_contexts
        .get("app/src/plain.rs")
        .ok_or("missing plain.rs file context")?;
    assert_eq!(plain_context.stack, vec!["plain"]);
    assert_eq!(plain_context.parent.as_deref(), Some("app/src/lib.rs"));
    assert!(index.symbols.iter().any(|symbol| symbol.name == "plain_fn"));
    index.validate_provenance()?;
    Ok(())
}

#[test]
fn inline_module_mod_edit_is_incrementally_equivalent() -> Result<(), Box<dyn Error>> {
    let tmp = make_app_with_path_and_nested_mods()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Add a file-backed mod inside the inline block: the dirty crate_root.rs
    // re-derives the new file under the same nested stack.
    let crate_root_path = root.join("app/src/core/crate_root.rs");
    let source = fs::read_to_string(&crate_root_path)?;
    let edited = source.replace("pub mod ser;", "pub mod ser;\n    pub mod extra;");
    fs::write(&crate_root_path, edited)?;
    write_file(
        &root.join("app/src/core/extra.rs"),
        "pub fn extra_fn() -> u8 { 2 }\n",
    )?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        index.symbols.iter().any(|symbol| symbol.name == "extra_fn"),
        "new nested module file must be extracted incrementally"
    );
    assert_eq!(
        index
            .file_contexts
            .get("app/src/core/extra.rs")
            .and_then(|record| record.parent.as_deref()),
        Some("app/src/core/crate_root.rs")
    );
    assert_eq!(
        index
            .file_contexts
            .get("app/src/core/extra.rs")
            .map(|record| record.stack.clone()),
        Some(vec![
            "crate_root".to_string(),
            "lib".to_string(),
            "extra".to_string()
        ])
    );
    assert_equivalent_to_full_rebuild(&index, root, true)?;

    // Remove the mod declaration and delete the file: the subtree drop must
    // remove the nested module even though the parent file is the dirty one.
    let source = fs::read_to_string(&crate_root_path)?;
    let edited = source.replace("\n    pub mod extra;", "");
    fs::write(&crate_root_path, edited)?;
    fs::remove_file(root.join("app/src/core/extra.rs"))?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        index.symbols.iter().all(|symbol| symbol.name != "extra_fn"),
        "removed nested module must be dropped incrementally"
    );
    assert!(!index.file_contexts.contains_key("app/src/core/extra.rs"));
    assert_equivalent_to_full_rebuild(&index, root, true)?;
    Ok(())
}

#[test]
fn nested_module_file_edit_is_incrementally_equivalent() -> Result<(), Box<dyn Error>> {
    let tmp = make_app_with_path_and_nested_mods()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Editing the nested module file itself re-extracts it under its existing
    // stack while keeping the module parent linkage.
    fs::write(
        root.join("app/src/core/ser/mod.rs"),
        "pub trait Serializer {\n    fn serialize(&self) -> u8;\n}\n\npub fn ser_extra() -> u8 { 3 }\n",
    )?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "ser_extra")
    );
    assert_eq!(
        index
            .file_contexts
            .get("app/src/core/ser/mod.rs")
            .map(|record| record.stack.clone()),
        Some(vec![
            "crate_root".to_string(),
            "lib".to_string(),
            "ser".to_string()
        ])
    );
    assert_eq!(
        index
            .file_contexts
            .get("app/src/core/ser/mod.rs")
            .and_then(|record| record.parent.as_deref()),
        Some("app/src/core/crate_root.rs")
    );
    assert_equivalent_to_full_rebuild(&index, root, true)?;
    Ok(())
}

/// Serde-shaped fixture: `serde/src/core` is a symlink to `serde_core/src`,
/// and both crates declare the same `crate_root` file, whose inline
/// `mod lib { }` block declares the file-backed `mod ser;`. The shared file
/// must be extracted exactly once despite being reachable from both packages.
#[cfg(unix)]
fn make_symlinked_serde_workspace() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let root = tempdir()?;
    write_file(
        &root.path().join("Cargo.toml"),
        r#"
[workspace]
members = ["serde", "serde_core"]

[workspace.package]
edition = "2024"
"#,
    )?;
    fs::create_dir_all(root.path().join("serde/src"))?;
    fs::create_dir_all(root.path().join("serde_core/src/ser"))?;
    write_file(
        &root.path().join("serde/Cargo.toml"),
        r#"
[package]
name = "serde"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        &root.path().join("serde/src/lib.rs"),
        r#"
#[path = "core/crate_root.rs"]
mod crate_root;
"#,
    )?;
    std::os::unix::fs::symlink("../../serde_core/src", root.path().join("serde/src/core"))?;
    write_file(
        &root.path().join("serde_core/Cargo.toml"),
        r#"
[package]
name = "serde_core"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        &root.path().join("serde_core/src/lib.rs"),
        "mod crate_root;\n",
    )?;
    write_file(
        &root.path().join("serde_core/src/crate_root.rs"),
        "mod lib {\n    pub mod ser;\n}\n",
    )?;
    write_file(
        &root.path().join("serde_core/src/ser/mod.rs"),
        "pub trait Serializer {\n    fn serialize(&self) -> u8;\n}\n",
    )?;
    init_git(root.path())?;
    Ok(root)
}

#[cfg(unix)]
#[test]
fn symlinked_shared_module_is_extracted_once_and_equivalent() -> Result<(), Box<dyn Error>> {
    let root = make_symlinked_serde_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root_path = root.path();

    let (index, mode) = build_or_update(&index_path, &candidates_path, root_path)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| { symbol.name == "Serializer" && symbol.kind == SymbolKind::Trait }),
        "Serializer must be indexed through the symlinked #[path] module"
    );
    // The shared crate_root file and the nested ser module are each extracted
    // exactly once despite both packages walking them.
    assert_eq!(
        index
            .file_contexts
            .keys()
            .filter(|key| key.ends_with("crate_root.rs"))
            .count(),
        1,
        "crate_root.rs must be extracted exactly once"
    );
    assert_eq!(
        index
            .file_contexts
            .keys()
            .filter(|key| key.ends_with("ser/mod.rs"))
            .count(),
        1,
        "ser/mod.rs must be extracted exactly once"
    );
    let ser_context = index
        .file_contexts
        .get("serde_core/src/ser/mod.rs")
        .ok_or("missing serde_core/src/ser/mod.rs file context")?;
    assert_eq!(ser_context.stack, vec!["crate_root", "lib", "ser"]);
    assert_eq!(
        ser_context.parent.as_deref(),
        Some("serde_core/src/crate_root.rs")
    );

    // Incremental edits to the shared file stay equivalent to a full rebuild.
    fs::write(
        root_path.join("serde_core/src/ser/mod.rs"),
        "pub trait Serializer {\n    fn serialize(&self) -> u8;\n}\n\npub fn shared_extra() -> u8 { 4 }\n",
    )?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root_path)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "shared_extra")
    );
    assert_equivalent_to_full_rebuild(&index, root_path, true)?;
    Ok(())
}
