//! Repository code intelligence index.

use std::fs;
use std::io::BufReader;
use std::path::Path;

use serde_json::{from_reader, to_vec_pretty};

mod builder;
mod error;
mod identity;
mod metadata;
mod query;
mod symbols;
mod types;

pub use error::CodeIntelError;
pub use types::{
    CodeIndexSummary, CodeQuery, DependencyRecord, PackageRecord, QueryResult, QuerySummary,
    RecordProvenance, RepositoryCodeIndex, SourceRange, SymbolKind, SymbolMarkers, SymbolRecord,
    TargetRecord, Visibility,
};

impl RepositoryCodeIndex {
    /// Save index to JSON without exposing a partially written prior index.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), CodeIntelError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| CodeIntelError::Persist {
                context: "create parent directory".to_string(),
                details: error.to_string(),
            })?;
        }
        let bytes = to_vec_pretty(self).map_err(|error| CodeIntelError::Persist {
            context: "serialize index".to_string(),
            details: error.to_string(),
        })?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, bytes).map_err(|error| CodeIntelError::Persist {
            context: "write temporary index file".to_string(),
            details: format!("{temporary:?}: {error}"),
        })?;
        fs::rename(&temporary, path).map_err(|error| CodeIntelError::Persist {
            context: "atomically replace index file".to_string(),
            details: format!("{path:?}: {error}"),
        })
    }

    /// Load index from JSON.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CodeIntelError> {
        let path = path.as_ref();
        let file = fs::File::open(path).map_err(|error| CodeIntelError::Persist {
            context: "open index file".to_string(),
            details: format!("{path:?}: {error}", path = path),
        })?;
        let reader = BufReader::new(file);
        from_reader(reader).map_err(|error| CodeIntelError::Persist {
            context: "deserialize index".to_string(),
            details: error.to_string(),
        })
    }

    /// Query extracted symbols.
    pub fn query(&self, query: CodeQuery, limit: usize) -> QueryResult {
        symbols::query_symbols(&self.symbols, query, limit)
    }

    /// Whether stored parser generation matches `parser_generation`.
    pub fn is_stale_generation(&self, parser_generation: &str) -> bool {
        self.summary.parser_generation != parser_generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

#[cfg(test)]
mod tests {
    #[test]
    fn simple_test() {
        assert_eq!(crate_one::math::add(2, 3), 5);
    }
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

    #[test]
    fn build_collects_workspace_metadata() -> Result<(), Box<dyn Error>> {
        let tmp = make_workspace()?;
        let index = build_index(tmp.path(), "g1")?;

        assert_eq!(index.summary.package_count, 1);
        assert!(
            index
                .symbols
                .iter()
                .any(|symbol| symbol.name == "Widget" && matches!(symbol.kind, SymbolKind::Struct))
        );
        let widget = index
            .symbols
            .iter()
            .find(|symbol| symbol.name == "Widget")
            .ok_or("missing Widget symbol")?;
        assert!(!widget.is_test);
        let test_function = index
            .symbols
            .iter()
            .find(|symbol| symbol.name == "simple_test")
            .ok_or("missing simple_test symbol")?;
        assert!(test_function.is_test);
        Ok(())
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
        Ok(())
    }

    #[test]
    fn markers_capture_axum_routes_and_sqlx_queries() -> Result<(), Box<dyn Error>> {
        let tmp = make_workspace_with_routes()?;
        let index = build_index(tmp.path(), "g3")?;

        let routed =
            match index.symbols.iter().find(|symbol| {
                symbol.name == "routed" && matches!(symbol.kind, SymbolKind::Function)
            }) {
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
}
