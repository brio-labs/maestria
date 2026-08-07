//! Repository code intelligence index.

//! Responsibility map:
//! - `builder`: repository index construction and exclusion handling.
//! - `changes`: git-history-aware change delta computation for queries.
//! - `context`: public repository-context query models and execution.
//! - `context_assembly`: deterministic context graph result assembly.
//! - `context_support`: context query normalization and bounded graph traversal.
//! - `error`: typed indexing, persistence, and integrity errors.
//! - `freshness`: repository snapshot freshness comparison.
//! - `identity`: repository and worktree provenance discovery.
//! - `incremental`: git-incremental index rebuild (assemble, candidates,
//!   reconcile, and state submodules).
//! - `markers`: validated code-marker records and the marker query surface.
//! - `metadata`: bounded Cargo workspace metadata extraction.
//! - `provenance`: canonical per-file content hashing and hash validation.
//! - `query`: bounded in-memory symbol query execution.
//! - `references`: cross-file symbol references over persisted relations.
//! - `symbols`: Rust source symbol and relation extraction.
//! - `types`: serializable index, symbol, relation, and query records.
//! - `index`: index persistence, querying, and provenance validation.
//! - `language`: language backend boundary (Rust and Python backends).
//! - `walk`: bounded repository file walking for identity and discovery.

mod builder;
mod changes;
mod context;
mod context_assembly;
mod context_support;
mod error;
mod freshness;
mod identity;
mod incremental;
mod language;
mod markers;
mod metadata;
mod provenance;
mod query;
mod references;
mod symbols;
mod types;
mod walk;
pub use changes::is_plausible_commit_sha;
pub use context::{
    ContextDirection, MAX_CONTEXT_DEPTH, RepositoryContextEdge, RepositoryContextNode,
    RepositoryContextQuery, RepositoryContextResult, RepositoryContextSummary,
};
pub use error::CodeIntelError;
pub use freshness::{RepositoryFreshness, RepositoryIdentitySnapshot};
pub use markers::{
    CodeMarker, CodeMarkerError, CodeMarkerKind, MarkerQueryKind, MarkerQueryKindParseError,
};
pub use references::ReferencesDirectionParseError;
pub use types::{
    CodeIndexSummary, CodeQuery, CodeRelationKind, CodeRelationRecord, CodeRelationSummary,
    CommitSha, DependencyRecord, FileContextRecord, PackageRecord, ParserGeneration, QueryResult,
    QuerySummary, RecordProvenance, ReferencesDirection, RelationSourceAvailability,
    RelationSourceKind, RelationSourceStatus, RepositoryChangeDelta, RepositoryCodeIndex,
    SourceRange, SymbolKind, SymbolMarkers, SymbolRecord, TargetRecord, Visibility,
    WorktreeIdentity,
};

mod index;
pub use incremental::{
    REPOSITORY_CODE_CANDIDATES_FILENAME, RepositoryIndexBuildMode, build_or_update_repository_index,
};
pub use index::{REPOSITORY_CODE_INDEX_FILENAME, REPOSITORY_CODE_PARSER_GENERATION};
