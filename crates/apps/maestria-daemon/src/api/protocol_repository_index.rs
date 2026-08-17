//! Wire types for the repository code index operations.

use maestria_code_intel::CodeIndexSummary;
use maestria_index_selection::candidates::CandidateDir;
use std::collections::BTreeMap;

/// The classified candidate tree for a repository root.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexCandidatesResponse {
    pub root: String,
    pub tree: CandidateDir,
}

/// The persisted repository selection profile, when one exists.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexSelectionResponse {
    pub profile: Option<maestria_index_selection::profile::IndexSelectionProfile>,
}

/// The repository code index summary, reduced to wire-safe fields.
///
/// The full [`CodeIndexSummary`] carries the changed-symbol record payload,
/// which exceeds the daemon protocol frame on real repositories; the wire
/// carries the counts instead. Serialized without the summary's
/// `relation_summary` and `changed` payloads.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexSummary {
    pub repository_root: String,
    pub commit_sha: String,
    pub worktree_identity: String,
    pub parser_generation: String,
    pub package_count: usize,
    pub target_count: usize,
    pub symbol_count: usize,
    pub file_count: usize,
    pub packages: Vec<String>,
    pub workspace_warnings: Vec<String>,
    pub changed_files: usize,
    pub changed_symbols: usize,
    #[serde(default)]
    pub selected_paths: Vec<String>,
    #[serde(default)]
    pub selection_policies: BTreeMap<String, maestria_index_selection::IndexPolicy>,
}

impl RepositoryIndexSummary {
    pub(crate) fn from_index(summary: &CodeIndexSummary) -> Self {
        Self {
            repository_root: summary.repository_root.clone(),
            commit_sha: summary.commit_sha.0.clone(),
            worktree_identity: summary.worktree_identity.0.clone(),
            parser_generation: summary.parser_generation.0.clone(),
            package_count: summary.package_count,
            target_count: summary.target_count,
            symbol_count: summary.symbol_count,
            file_count: summary.file_count,
            packages: summary.packages.clone(),
            workspace_warnings: summary.workspace_warnings.clone(),
            changed_files: summary.changed.files().len(),
            changed_symbols: summary.changed.symbols().len(),
            selected_paths: summary.selected_paths.clone(),
            selection_policies: summary.selection_policies.clone(),
        }
    }
}

/// One direct file of a repository directory, as shown by the browser.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexFile {
    /// Repository-relative path of the file.
    pub path: String,
    pub size: u64,
    /// Population bucket: `code`, `doc`, `manifest`, or `other`.
    pub kind: String,
}

/// The direct subdirectories of one repository directory, each classified
/// with empty children (fetched on demand), bounded for the wire.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexChildrenResponse {
    pub root: String,
    pub path: String,
    pub children: Vec<CandidateDir>,
}

/// The direct files of one repository directory, bounded for the wire.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexFilesResponse {
    pub root: String,
    pub path: String,
    pub files: Vec<RepositoryIndexFile>,
    /// Whether more files exist than the response carries.
    pub truncated: bool,
}

/// The outcome of a repository code index run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexRunResponse {
    pub mode: String,
    pub summary: RepositoryIndexSummary,
    /// Canonical source artifacts registered through the kernel.
    pub registered: usize,
    /// Sources skipped during registration (already indexed, empty, or
    /// secret-like content).
    pub skipped: usize,
}

/// Live progress of the active repository index run, when one is running.
/// `registered` counts expected sources already processed (submitted or
/// skipped) out of `total`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexProgress {
    /// `building` while extraction runs, `registering` while sources are
    /// durably registered.
    pub phase: String,
    pub total: usize,
    pub registered: usize,
}

/// The live progress of the active repository index run. A lightweight
/// read for polling: no index load, no git calls.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexProgressResponse {
    pub progress: Option<RepositoryIndexProgress>,
}

/// The persisted repository code index status for a root.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexStatusResponse {
    pub root: String,
    pub present: bool,
    pub summary: Option<RepositoryIndexSummary>,
    pub freshness: Option<maestria_code_intel::RepositoryFreshness>,
    #[serde(default)]
    pub progress: Option<RepositoryIndexProgress>,
}
