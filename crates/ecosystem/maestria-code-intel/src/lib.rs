//! Repository code intelligence index.

//! Responsibility map:
//! - `builder`: repository index construction and exclusion handling.
//! - `context`: public repository-context query models and execution.
//! - `context_assembly`: deterministic context graph result assembly.
//! - `context_support`: context query normalization and bounded graph traversal.
//! - `error`: typed indexing, persistence, and integrity errors.
//! - `freshness`: repository snapshot freshness comparison.
//! - `identity`: repository and worktree provenance discovery.
//! - `metadata`: bounded Cargo workspace metadata extraction.
//! - `query`: bounded in-memory symbol query execution.
//! - `symbols`: Rust source symbol and relation extraction.
//! - `types`: serializable index, symbol, relation, and query records.
//! - `index`: index persistence, querying, and provenance validation.

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

mod index;
pub use index::{REPOSITORY_CODE_INDEX_FILENAME, REPOSITORY_CODE_PARSER_GENERATION};
