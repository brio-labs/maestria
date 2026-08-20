//! Typed wire DTOs for the repository code index operations.

use serde::{Deserialize, Serialize};

use crate::index_types::CandidateDirWire;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexCandidatesWire {
    pub root: String,
    pub tree: CandidateDirWire,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexSummaryWire {
    pub repository_root: String,
    pub commit_sha: String,
    pub worktree_identity: String,
    pub parser_generation: String,
    pub package_count: usize,
    pub symbol_count: usize,
    pub file_count: usize,
    #[serde(default)]
    pub selected_paths: Vec<String>,
    pub changed_files: usize,
    pub changed_symbols: usize,
    #[serde(default)]
    pub workspace_warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexRunInputWire {
    pub root: String,
    pub includes: Vec<String>,
    pub policies: std::collections::BTreeMap<String, crate::index_types::IndexPolicyWire>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexRunWire {
    pub mode: String,
    pub summary: RepositoryIndexSummaryWire,
    pub registered: usize,
    pub skipped: usize,
}

/// One identity snapshot inside a freshness verdict.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIdentitySnapshotWire {
    pub commit_sha: String,
    pub worktree_identity: String,
}

/// The freshness verdict for a persisted repository code index.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum RepositoryFreshnessWire {
    Current {
        indexed: RepositoryIdentitySnapshotWire,
        current: RepositoryIdentitySnapshotWire,
    },
    Stale {
        indexed: RepositoryIdentitySnapshotWire,
        current: RepositoryIdentitySnapshotWire,
    },
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexProgressWire {
    pub phase: String,
    pub total: usize,
    pub registered: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexProgressResponseWire {
    pub progress: Option<RepositoryIndexProgressWire>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexStatusWire {
    pub root: String,
    pub present: bool,
    pub summary: Option<RepositoryIndexSummaryWire>,
    pub freshness: Option<RepositoryFreshnessWire>,
    #[serde(default)]
    pub progress: Option<RepositoryIndexProgressWire>,
}

/// One direct file of a repository directory.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexFileWire {
    pub path: String,
    pub size: u64,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexChildrenWire {
    pub root: String,
    pub path: String,
    pub children: Vec<crate::index_types::CandidateDirWire>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexFilesWire {
    pub root: String,
    pub path: String,
    pub files: Vec<RepositoryIndexFileWire>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepositoryIndexBrowseInputWire {
    pub root: String,
    pub path: String,
}
