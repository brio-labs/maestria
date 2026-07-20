//! Repository code intelligence index.

//! Responsibility map:
//! - `builder`: module responsibility.
//! - `context`: module responsibility.
//! - `context_assembly`: module responsibility.
//! - `context_support`: module responsibility.
//! - `error`: module responsibility.
//! - `freshness`: module responsibility.
//! - `identity`: module responsibility.
//! - `metadata`: module responsibility.
//! - `query`: module responsibility.
//! - `symbols`: module responsibility.
//! - `types`: module responsibility.
/// Persisted filename for the repository code projection.
pub const REPOSITORY_CODE_INDEX_FILENAME: &str = "repository-code-index.json";

/// Parser generation used by the current Rust repository projection.
pub const REPOSITORY_CODE_PARSER_GENERATION: &str = "cargo-rust-code-v1";
use std::fs;
use std::io::BufReader;
use std::path::{Component, Path};

use serde_json::{from_reader, to_vec_pretty};

mod builder;
mod context;
mod context_assembly;
mod context_support;
mod error;
mod freshness;
mod identity;
mod metadata;
mod query;
mod symbols;
mod types;

pub use context::{
    ContextDirection, RepositoryContextEdge, RepositoryContextNode, RepositoryContextQuery,
    RepositoryContextResult, RepositoryContextSummary,
};
pub use error::CodeIntelError;
pub use freshness::{RepositoryFreshness, RepositoryIdentitySnapshot};
pub use types::{
    CodeIndexSummary, CodeQuery, CodeRelationKind, CodeRelationRecord, CodeRelationSummary,
    DependencyRecord, PackageRecord, QueryResult, QuerySummary, RecordProvenance,
    RelationSourceAvailability, RelationSourceKind, RelationSourceStatus, RepositoryCodeIndex,
    SourceRange, SymbolKind, SymbolMarkers, SymbolRecord, TargetRecord, Visibility,
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
    /// Validate that all persisted records belong to this index snapshot.
    pub fn validate_provenance(&self) -> Result<(), CodeIntelError> {
        let summary = &self.summary;
        if summary.package_count != self.packages.len()
            || summary.symbol_count != self.symbols.len()
            || summary.target_count
                != self
                    .packages
                    .iter()
                    .map(|package| package.targets.len())
                    .sum::<usize>()
            || summary.relation_summary.total_relations != self.relations.len()
        {
            return Err(CodeIntelError::Integrity {
                context: "index summary counts".to_string(),
                details: "persisted counts do not match records".to_string(),
            });
        }

        for package in &self.packages {
            validate_record_provenance(summary, &package.provenance, "package")?;
            for dependency in &package.dependencies {
                validate_record_provenance(summary, &dependency.provenance, "dependency")?;
            }
            for target in &package.targets {
                validate_record_provenance(summary, &target.provenance, "target")?;
            }
        }
        for symbol in &self.symbols {
            validate_record_provenance(summary, &symbol.provenance, "symbol")?;
        }

        let symbols = self
            .symbols
            .iter()
            .map(|symbol| (symbol.record_id.as_str(), symbol))
            .collect::<std::collections::BTreeMap<_, _>>();
        for relation in &self.relations {
            if relation.parser_generation != summary.parser_generation {
                return Err(CodeIntelError::Integrity {
                    context: "relation parser generation".to_string(),
                    details: relation.parser_generation.clone(),
                });
            }
            let Some(source) = symbols.get(relation.source_record_id.as_str()) else {
                return Err(CodeIntelError::Integrity {
                    context: "relation source endpoint".to_string(),
                    details: relation.source_record_id.clone(),
                });
            };
            let Some(target) = symbols.get(relation.target_record_id.as_str()) else {
                return Err(CodeIntelError::Integrity {
                    context: "relation target endpoint".to_string(),
                    details: relation.target_record_id.clone(),
                });
            };
            if relation.source_provenance != source.provenance
                || relation.target_provenance != target.provenance
            {
                return Err(CodeIntelError::Integrity {
                    context: "relation endpoint provenance".to_string(),
                    details: format!(
                        "{} -> {}",
                        relation.source_record_id, relation.target_record_id
                    ),
                });
            }
        }
        Ok(())
    }
}

fn validate_record_provenance(
    summary: &CodeIndexSummary,
    provenance: &RecordProvenance,
    record_kind: &str,
) -> Result<(), CodeIntelError> {
    let source_path = Path::new(&provenance.file_path);
    if source_path.is_absolute()
        || source_path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(CodeIntelError::Integrity {
            context: format!("{record_kind} source path"),
            details: provenance.file_path.clone(),
        });
    }
    if provenance.repository_root != summary.repository_root
        || provenance.commit_sha != summary.commit_sha
        || provenance.worktree_identity != summary.worktree_identity
        || provenance.parser_generation != summary.parser_generation
    {
        return Err(CodeIntelError::Integrity {
            context: format!("{record_kind} provenance"),
            details: provenance.file_path.clone(),
        });
    }
    Ok(())
}
