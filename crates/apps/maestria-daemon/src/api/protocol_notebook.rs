use super::{CoverageResponse, EvidenceResponse};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookListResponse {
    pub notebooks: Vec<NotebookSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookSummary {
    pub notebook_id: u64,
    pub title: String,
    pub source_count: usize,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookResponse {
    pub notebook_id: u64,
    pub title: String,
    pub sources: Vec<NotebookSourceSelection>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookSourceSelection {
    pub source_key: String,
    pub available: bool,
    pub artifact_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookSourceCatalogResponse {
    pub sources: Vec<NotebookSourceCatalogEntry>,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookSourceCatalogEntry {
    pub source_key: String,
    pub artifact_id: Option<u64>,
    pub title: Option<String>,
    pub content_hash: Option<String>,
    pub index_status: String,
    pub parse_status: Option<String>,
    pub source_kind: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookContextResponse {
    pub query: String,
    pub query_id: u64,
    pub trace_id: u64,
    pub source_selection_digest: Option<String>,
    pub index_generation: u64,
    pub fingerprint: String,
    pub answerability: String,
    pub coverage: CoverageResponse,
    pub gaps: Vec<String>,
    pub citations: Vec<NotebookCitationResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookCitationResponse {
    pub rank: usize,
    pub score: i64,
    pub evidence: EvidenceResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookDraftListResponse {
    pub drafts: Vec<NotebookDraftSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookDraftSummary {
    pub draft_id: u64,
    pub title: String,
    pub revision: u64,
    pub citation_count: usize,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookDraftResponse {
    pub draft_id: u64,
    pub notebook_id: u64,
    pub title: String,
    pub markdown: String,
    pub body_hash: String,
    pub revision: u64,
    pub citations: Vec<FrozenNotebookCitationResponse>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenNotebookCitationResponse {
    pub evidence_id: u64,
    pub artifact_id: u64,
    pub artifact_title: String,
    pub artifact_content_hash: String,
    pub source: String,
    pub excerpt: String,
    pub observed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookDraftSavedResponse {
    pub draft_id: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookDraftDeletedResponse {
    pub notebook_id: u64,
    pub draft_id: u64,
    pub revision: u64,
}
