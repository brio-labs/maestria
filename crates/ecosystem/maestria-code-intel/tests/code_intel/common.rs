use maestria_code_intel::*;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

pub fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn Error>> {
    fs::write(path, contents)?;
    Ok(())
}

pub fn init_git(repo_root: &Path) -> Result<(), Box<dyn Error>> {
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

pub fn run_git(repo_root: &Path, args: &[&str], operation: &str) -> Result<(), Box<dyn Error>> {
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

pub fn build_index(
    root: &Path,
    parser_generation: &str,
) -> Result<RepositoryCodeIndex, Box<dyn Error>> {
    Ok(RepositoryCodeIndex::build(root, parser_generation)?)
}

pub fn make_workspace() -> Result<tempfile::TempDir, Box<dyn Error>> {
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

pub fn make_workspace_with_routes() -> Result<tempfile::TempDir, Box<dyn Error>> {
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

pub fn relation_lib_source() -> String {
    [relation_lib_head(), relation_lib_tail()].concat()
}

pub fn relation_lib_head() -> &'static str {
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

pub fn relation_lib_tail() -> &'static str {
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

pub fn make_workspace_with_relations() -> Result<tempfile::TempDir, Box<dyn Error>> {
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

pub fn symbol_id<'a>(
    symbols: &'a [SymbolRecord],
    qualified_name: &str,
) -> Result<&'a str, Box<dyn Error>> {
    symbols
        .iter()
        .find(|symbol| symbol.qualified_name == qualified_name)
        .map(|symbol| symbol.record_id.as_str())
        .ok_or_else(|| format!("missing symbol {qualified_name}").into())
}

pub fn relation_sort_key(
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
