use std::fs;
use std::io::BufReader;
use std::path::{Component, Path};

use maestria_domain::ContentHash;
use serde_json::{from_reader, to_vec_pretty};

use crate::error::CodeIntelError;
use crate::symbols;
use crate::types::{
    CodeIndexSummary, CodeQuery, QueryResult, RecordProvenance, RepositoryCodeIndex,
};

/// Persisted filename for the repository code projection.
pub const REPOSITORY_CODE_INDEX_FILENAME: &str = "repository-code-index.json";
pub const REPOSITORY_CODE_PARSER_GENERATION: &str = "repository-code-v4";

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
        let index: Self = from_reader(reader).map_err(|error| CodeIntelError::Persist {
            context: "deserialize index".to_string(),
            details: error.to_string(),
        })?;
        index.validate_provenance()?;
        Ok(index)
    }

    /// Query extracted symbols through the caller's authorization policy.
    ///
    /// A `CodeQuery::Changed` query resolves its changed file set here: the
    /// persisted build-time delta when `since` is `None`, or a live git diff
    /// plus the current dirty set when `Some`. Live git failures surface as
    /// the caller's error type (`E: From<CodeIntelError>`).
    pub fn query<E, F>(
        &self,
        query: CodeQuery,
        limit: usize,
        mut authorize: F,
    ) -> Result<QueryResult, E>
    where
        F: FnMut(&crate::SymbolRecord) -> Result<bool, E>,
        E: From<CodeIntelError>,
    {
        let changed_files = match &query {
            CodeQuery::Changed { since } => {
                Some(crate::changes::changed_file_set(self, since.as_ref())?)
            }
            _ => None,
        };
        symbols::query_symbols(
            &self.symbols,
            query,
            limit,
            changed_files.as_ref(),
            &mut authorize,
        )
    }

    /// Whether stored parser generation matches `parser_generation`.
    pub fn is_stale_generation(&self, parser_generation: &str) -> bool {
        self.summary.parser_generation.as_str() != parser_generation
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
        if !self.file_contexts.is_empty() {
            for symbol in &self.symbols {
                if !self
                    .file_contexts
                    .contains_key(&symbol.provenance.file_path)
                {
                    return Err(CodeIntelError::Integrity {
                        context: "index file contexts".to_string(),
                        details: symbol.provenance.file_path.clone(),
                    });
                }
            }
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
                    details: relation.parser_generation.to_string(),
                });
            }
            if relation.confidence_milli > 1000 {
                return Err(CodeIntelError::Integrity {
                    context: "relation confidence range".to_string(),
                    details: format!(
                        "{} -> {} confidence {}",
                        relation.source_record_id,
                        relation.target_record_id,
                        relation.confidence_milli
                    ),
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
    if !ContentHash::is_well_formed(&provenance.content_hash) {
        return Err(CodeIntelError::Integrity {
            context: format!("{record_kind} content hash"),
            details: provenance.content_hash.clone(),
        });
    }
    if provenance.file_path.is_empty()
        || provenance.source_range.start_line() == 0
        || provenance.source_range.end_line() < provenance.source_range.start_line()
    {
        return Err(CodeIntelError::Integrity {
            context: format!("{record_kind} source range"),
            details: provenance.file_path.clone(),
        });
    }
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
