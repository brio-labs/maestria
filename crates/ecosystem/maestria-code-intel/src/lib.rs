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
}
