//! Shared serializable types for repository code intelligence records.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Byte/line provenance for every extracted record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    /// First source line (1-indexed).
    pub start_line: usize,
    /// Last source line (1-indexed).
    pub end_line: usize,
}

/// Repository identity attached to every persisted record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordProvenance {
    /// Repository root used for this index build.
    pub repository_root: String,
    /// `git rev-parse HEAD` output.
    pub commit_sha: String,
    /// Deterministic hash derived from tracked file contents/paths.
    pub worktree_identity: String,
    /// Relative file path from repository root.
    pub file_path: String,
    /// Source span for this record.
    pub source_range: SourceRange,
    /// Parser generation passed at index build time.
    pub parser_generation: String,
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
    pub parser_generation: String,
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
}

/// Query summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySummary {
    pub query: CodeQuery,
    pub matched: usize,
    pub returned: usize,
    pub truncated: bool,
    pub limit: usize,
    pub regex_error: Option<String>,
}

/// Query output payload suitable for CLI JSON rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryResult {
    pub summary: QuerySummary,
    pub records: Vec<SymbolRecord>,
}

/// Top-level index summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeIndexSummary {
    pub repository_root: String,
    pub commit_sha: String,
    pub worktree_identity: String,
    pub parser_generation: String,
    pub package_count: usize,
    pub target_count: usize,
    pub symbol_count: usize,
    pub file_count: usize,
    pub packages: Vec<String>,
    /// Privacy exclusions applied to source identity and extraction.
    pub excluded_patterns: Vec<String>,
    /// Relation extraction status and summary.
    #[serde(default)]
    pub relation_summary: CodeRelationSummary,
}

/// Serializable persisted index container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryCodeIndex {
    pub summary: CodeIndexSummary,
    pub packages: Vec<PackageRecord>,
    pub symbols: Vec<SymbolRecord>,
    #[serde(default)]
    pub relations: Vec<CodeRelationRecord>,
}
