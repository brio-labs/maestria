//! Wire types for the index choice operations.

use maestria_index_selection::candidates::CandidateDir;

/// The candidate tree for a root, as scanned by the choice layer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexCandidatesResponse {
    pub root: String,
    pub home_root: bool,
    pub tree: CandidateDir,
}

/// The persisted selection profile, when one exists.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexSelectionResponse {
    pub profile: Option<maestria_index_selection::profile::IndexSelectionProfile>,
}

/// The outcome of an index run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexRunResponse {
    pub submitted: usize,
    pub skipped: usize,
}
