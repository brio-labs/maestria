//! Typed wire DTOs for the index choice operations.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IndexCandidatesWire {
    pub root: String,
    #[serde(default)]
    pub home_root: bool,
    pub tree: CandidateDirWire,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CandidateDirWire {
    pub path: String,
    pub class: String,
    pub policy: IndexPolicyWire,
    pub file_count: usize,
    pub total_bytes: u64,
    #[serde(default)]
    pub children: Vec<CandidateDirWire>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct IndexPolicyWire {
    pub max_file_bytes: u64,
    pub skip_generated: bool,
    pub skip_minified: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct IndexSelectionProfileWire {
    pub root: String,
    #[serde(default)]
    pub includes: Vec<String>,
    #[serde(default)]
    pub policies: std::collections::BTreeMap<String, IndexPolicyWire>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IndexSelectionResponseWire {
    pub profile: Option<IndexSelectionProfileWire>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IndexRunWire {
    pub submitted: usize,
    pub skipped: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IndexRunInputWire {
    pub root: String,
    pub includes: Vec<String>,
    pub policies: std::collections::BTreeMap<String, IndexPolicyWire>,
}
