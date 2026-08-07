//! Shared serializable types for repository code intelligence records.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Byte/line provenance for every extracted record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SourceRangeDto")]
pub struct SourceRange {
    start_line: usize,
    end_line: usize,
}

impl SourceRange {
    /// Builds a one-based, inclusive source line range.
    pub fn new(start_line: usize, end_line: usize) -> Result<Self, SourceRangeError> {
        if start_line == 0 {
            return Err(SourceRangeError::StartMustBePositive);
        }
        if start_line > end_line {
            return Err(SourceRangeError::StartAfterEnd {
                start_line,
                end_line,
            });
        }
        Ok(Self {
            start_line,
            end_line,
        })
    }

    pub const fn start_line(&self) -> usize {
        self.start_line
    }

    pub const fn end_line(&self) -> usize {
        self.end_line
    }
}

/// Failure while building a validated [`SourceRange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRangeError {
    StartMustBePositive,
    StartAfterEnd { start_line: usize, end_line: usize },
}

impl std::fmt::Display for SourceRangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartMustBePositive => write!(f, "source range start line must be positive"),
            Self::StartAfterEnd {
                start_line,
                end_line,
            } => write!(
                f,
                "source range start line {start_line} must not exceed end line {end_line}"
            ),
        }
    }
}

impl std::error::Error for SourceRangeError {}

#[derive(Deserialize)]
struct SourceRangeDto {
    start_line: usize,
    end_line: usize,
}

impl TryFrom<SourceRangeDto> for SourceRange {
    type Error = SourceRangeError;

    fn try_from(dto: SourceRangeDto) -> Result<Self, Self::Error> {
        Self::new(dto.start_line, dto.end_line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_range_rejects_invalid_lines() {
        assert_eq!(
            SourceRange::new(0, 1),
            Err(SourceRangeError::StartMustBePositive)
        );
        assert_eq!(
            SourceRange::new(3, 2),
            Err(SourceRangeError::StartAfterEnd {
                start_line: 3,
                end_line: 2
            })
        );
        assert!(SourceRange::new(1, 1).is_ok());
    }
}

/// Distinct repository identity components (R56): commit, worktree
/// identity, and parser generation are semantically distinct identifiers
/// that must not be swapped; each is a newtype so interchange does not
/// compile. They serialize transparently as their underlying strings, so
/// persisted index JSON is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CommitSha(pub String);

impl CommitSha {
    pub fn new(sha: impl Into<String>) -> Self {
        Self(sha.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CommitSha {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for CommitSha {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for CommitSha {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorktreeIdentity(pub String);

impl WorktreeIdentity {
    pub fn new(identity: impl Into<String>) -> Self {
        Self(identity.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for WorktreeIdentity {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for WorktreeIdentity {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for WorktreeIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ParserGeneration(pub String);

impl ParserGeneration {
    pub fn new(generation: impl Into<String>) -> Self {
        Self(generation.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ParserGeneration {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ParserGeneration {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for ParserGeneration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Extraction context persisted per indexed source file (incremental rebuild input).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContextRecord {
    pub package: String,
    pub target: String,
    pub is_test_target: bool,
    pub is_bench_target: bool,
    /// Module stack (module names joined) this file was extracted under.
    pub stack: Vec<String>,
    pub is_test: bool,
    pub is_bench: bool,
    /// Relative path of the file that declared this file via `mod`; `None` for target roots.
    pub parent: Option<String>,
}

/// Repository identity attached to every persisted record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordProvenance {
    /// Repository root used for this index build.
    pub repository_root: String,
    /// `git rev-parse HEAD` output.
    pub commit_sha: CommitSha,
    /// Deterministic identity of the indexed worktree contents and paths.
    pub worktree_identity: WorktreeIdentity,
    /// Deterministic SHA-256 hash of the persisted source bytes (`sha256:<lowercase hex>`).
    pub content_hash: String,
    /// Relative file path from repository root.
    pub file_path: String,
    /// Source span for this record.
    pub source_range: SourceRange,
    /// Parser generation passed at index build time.
    pub parser_generation: ParserGeneration,
}

/// Visibility as represented by the Rust AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Crate,
    Super,
    Restricted,
    Private,
    Inherited,
}

/// Provenance-backed relation source kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationSourceKind {
    /// Relation discovered via `syn` AST extraction.
    Ast,
    /// Relation from rust-analyzer/LSP extraction.
    RustAnalyzer,
}

/// Reliability of a relation source lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationSourceAvailability {
    /// Lane is active and trustworthy for this index build.
    Available,
    /// Lane is intentionally unavailable and therefore degraded.
    Degraded,
    /// Lane failed and produced no relations.
    Unavailable,
}

/// Relation lane status persisted in the index summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationSourceStatus {
    pub source: RelationSourceKind,
    pub availability: RelationSourceAvailability,
    pub reason: Option<String>,
}

/// Relation kind persisted by AST extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodeRelationKind {
    Defines,
    Imports,
    Calls,
    Implements,
    Tests,
}

/// Public provenance-backed relation between two indexed symbols.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeRelationRecord {
    pub kind: CodeRelationKind,
    /// Source endpoint symbol record identifier.
    pub source_record_id: String,
    /// Target endpoint symbol record identifier.
    pub target_record_id: String,
    /// Source endpoint provenance snapshot.
    pub source_provenance: RecordProvenance,
    /// Target endpoint provenance snapshot.
    pub target_provenance: RecordProvenance,
    /// Parser generation that produced this relation.
    pub parser_generation: ParserGeneration,
    /// Relation-confidence in basis points (0-1000).
    pub confidence_milli: u16,
    /// Extractor type for this relation edge.
    pub source_kind: RelationSourceKind,
}

/// Relation summary for persisted index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CodeRelationSummary {
    pub total_relations: usize,
    #[serde(default)]
    pub source_statuses: Vec<RelationSourceStatus>,
}

/// Source-level markers extracted from AST/file context.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SymbolMarkers {
    /// File is `build.rs` or contains rerun instructions.
    pub build_script: bool,
    /// File is marked as generated.
    pub generated_code: bool,
    /// Axum routing macros found on this declaration.
    pub axum_routes: Vec<String>,
    /// SQLx query calls/macros detected in scope.
    pub sqlx_queries: Vec<String>,
}

/// Typed symbol kinds emitted by this index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Module,
    Struct,
    Enum,
    Union,
    Trait,
    TypeAlias,
    Function,
    Method,
    Impl,
    Const,
    Static,
    Import,
    Field,
    UnsafeBlock,
    Other,
}

/// Target metadata extracted from `cargo metadata`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetRecord {
    pub name: String,
    pub kind: Vec<String>,
    pub crate_types: Vec<String>,
    pub src_path: String,
    pub required_features: Vec<String>,
    pub doctest: bool,
    pub test: bool,
    pub bench: bool,
    pub doc: bool,
    pub provenance: RecordProvenance,
}

/// Dependency metadata extracted from `cargo metadata`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyRecord {
    pub name: String,
    pub package: Option<String>,
    pub source: Option<String>,
    pub version_req: String,
    pub kind: Vec<String>,
    pub optional: bool,
    pub uses_default_features: bool,
    pub features: Vec<String>,
    pub target: Option<String>,
    pub registry: Option<String>,
    pub provenance: RecordProvenance,
}

/// Package metadata extracted from workspace `cargo metadata`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageRecord {
    pub package_id: String,
    pub name: String,
    pub version: String,
    pub manifest_path: String,
    pub edition: Option<String>,
    pub authors: Vec<String>,
    pub source: Option<String>,
    pub description: Option<String>,
    pub features: BTreeMap<String, Vec<String>>,
    pub dependencies: Vec<DependencyRecord>,
    pub targets: Vec<TargetRecord>,
    pub provenance: RecordProvenance,
}

/// Single extracted declaration record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolRecord {
    pub record_id: String,
    pub package: String,
    pub target: String,
    pub kind: SymbolKind,
    pub name: String,
    pub qualified_name: String,
    pub visibility: Visibility,
    pub is_public_api: bool,
    pub is_async: bool,
    pub is_unsafe: bool,
    pub is_test: bool,
    pub is_bench: bool,
    pub signature: Option<String>,
    pub imports: Vec<String>,
    pub markers: SymbolMarkers,
    pub provenance: RecordProvenance,
}

/// Query description for in-memory filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CodeQuery {
    /// Match all symbols.
    All,
    /// Match symbol name or qualified name by substring.
    Symbol { pattern: String },
    /// Match file path by substring.
    Path { pattern: String },
    /// Match symbol, qualified symbol, or path by regex.
    Regex { pattern: String },
    /// Symbols in files that changed since `since`: the persisted build-time
    /// delta when `since` is `None`, or a live git diff plus the current
    /// dirty set when `Some`.
    Changed { since: Option<CommitSha> },
}

/// Query summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySummary {
    pub query: CodeQuery,
    pub matched: usize,
    pub returned: usize,
    pub truncated: bool,
    /// Number of indexed symbols examined before the bounded scan stopped.
    #[serde(default)]
    pub scanned: usize,
    pub limit: usize,
    pub regex_error: Option<String>,
}

/// Query output payload suitable for CLI JSON rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryResult {
    pub summary: QuerySummary,
    pub records: Vec<SymbolRecord>,
}

/// Files and symbols that changed between the build-time baseline and the
/// current repository state, computed from porcelain status and git history
/// metadata only (zero content reads).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryChangeDelta {
    /// Relative repository paths in the changed set, sorted.
    pub files: Vec<String>,
    /// `record_id`s of symbols whose file is in `files`, ordered by file
    /// then qualified name.
    pub symbols: Vec<String>,
}

/// Top-level index summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeIndexSummary {
    pub repository_root: String,
    pub commit_sha: CommitSha,
    pub worktree_identity: WorktreeIdentity,
    pub parser_generation: ParserGeneration,
    pub package_count: usize,
    pub target_count: usize,
    pub symbol_count: usize,
    pub file_count: usize,
    pub packages: Vec<String>,
    /// Privacy exclusions applied to source identity and extraction.
    pub excluded_patterns: Vec<String>,
    /// Per-workspace discovery degradations (a nested manifest that failed
    /// `cargo metadata` was skipped). Required on every persisted index;
    /// indexes without it fail to load and are rebuilt from scratch.
    pub workspace_warnings: Vec<String>,
    /// Relation extraction status and summary.
    #[serde(default)]
    pub relation_summary: CodeRelationSummary,
    /// Files and symbols changed since the build-time baseline (porcelain
    /// dirty set plus the git diff between the replaced index's commit and
    /// HEAD). Required on every persisted index; indexes persisted without
    /// it fail to load and rebuild from scratch.
    pub changed: RepositoryChangeDelta,
}

/// Serializable persisted index container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryCodeIndex {
    pub summary: CodeIndexSummary,
    pub packages: Vec<PackageRecord>,
    pub symbols: Vec<SymbolRecord>,
    #[serde(default)]
    pub relations: Vec<CodeRelationRecord>,
    /// Per-file extraction contexts keyed by relative path. Required on
    /// every persisted index; indexes without it fail to load and are
    /// rebuilt from scratch.
    pub file_contexts: BTreeMap<String, FileContextRecord>,
}
