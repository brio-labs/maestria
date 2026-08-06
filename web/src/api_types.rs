use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    pub type_uri: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientError {
    Problem(ProblemDetails),
    Network(String),
    InvalidResponse(String),
}

impl ClientError {
    pub fn title(&self) -> &str {
        match self {
            Self::Problem(problem) => &problem.title,
            Self::Network(_) => "Network error",
            Self::InvalidResponse(_) => "Invalid response",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Problem(problem) => problem.detail.clone(),
            Self::Network(detail) | Self::InvalidResponse(detail) => detail.clone(),
        }
    }

    pub fn problem_code(&self) -> Option<&str> {
        match self {
            Self::Problem(problem) => problem.type_uri.rsplit(':').next(),
            _ => None,
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.title(), self.detail())
    }
}
impl std::error::Error for ClientError {}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Envelope<T> {
    pub data: T,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct NotebookListPayload {
    pub notebooks: Vec<NotebookSummary>,
}

impl NotebookListPayload {
    pub fn into_vec(self) -> Vec<NotebookSummary> {
        self.notebooks
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NotebookSummary {
    pub notebook_id: u64,
    pub title: String,
    #[serde(default)]
    pub source_count: usize,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SourceSelection {
    pub source_key: String,
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub artifact_id: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Notebook {
    pub notebook_id: u64,
    pub title: String,
    #[serde(default)]
    pub source_count: usize,
    #[serde(default)]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub sources: Vec<SourceSelection>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CatalogSource {
    pub source_key: String,
    #[serde(default)]
    pub artifact_id: Option<u64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub index_status: String,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub available: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SourceCatalogWire {
    pub sources: Vec<CatalogSource>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DraftSummary {
    pub draft_id: u64,
    pub title: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SavedDraft {
    pub draft_id: u64,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DraftListWire {
    pub drafts: Vec<DraftSummary>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FrozenCitation {
    pub evidence_id: u64,
    pub artifact_id: u64,
    pub artifact_title: String,
    pub artifact_content_hash: String,
    pub source: serde_json::Value,
    pub excerpt: String,
    pub observed_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Draft {
    pub draft_id: u64,
    pub notebook_id: u64,
    pub title: String,
    pub markdown: String,
    pub revision: u64,
    #[serde(default)]
    pub citations: Vec<FrozenCitation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Evidence {
    pub evidence_id: u64,
    pub artifact_id: u64,
    pub artifact_title: String,
    #[serde(default)]
    pub artifact_content_hash: Option<String>,
    pub excerpt: String,
    pub observed_at: i64,
    #[serde(default)]
    pub source: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Citation {
    pub rank: u32,
    pub score: f64,
    pub evidence: Evidence,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct AskContext {
    #[serde(default)]
    pub answerability: Option<String>,
    #[serde(default)]
    pub coverage: Option<Coverage>,
    #[serde(default)]
    pub gaps: Vec<String>,
    #[serde(default)]
    pub citations: Vec<Citation>,
    #[serde(default)]
    pub trace_id: Option<u64>,
    #[serde(default)]
    pub query_id: Option<u64>,
    #[serde(default)]
    pub source_selection_digest: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Coverage {
    #[serde(default)]
    pub percent_covered: f64,
    #[serde(default)]
    pub distinct_sources: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Agent {
    pub id: String,
    pub label: String,
    pub status: String,
    #[serde(default)]
    pub config_options: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Bootstrap {
    #[serde(default)]
    pub notebooks: NotebookListPayload,
    #[serde(default)]
    pub agents: Vec<Agent>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AskResponse {
    pub answer_markdown: String,
    #[serde(default)]
    pub citations: Vec<Citation>,
    #[serde(default)]
    pub draft_previews: Vec<DraftPreview>,
    #[serde(default)]
    pub context: AskContext,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DraftPreview {
    pub title: String,
    pub markdown: String,
    #[serde(default)]
    pub citations: Vec<Citation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AskTurn {
    pub role: String,
    pub markdown: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AskRequest {
    pub question: String,
    pub history: Vec<AskTurn>,
    pub agent_id: String,
    pub config: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CreateDraft {
    pub title: String,
    pub markdown: String,
    pub evidence_ids: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct UpdateDraft {
    pub expected_revision: u64,
    pub title: String,
    pub markdown: String,
    pub evidence_ids: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DeleteDraft {
    pub expected_revision: u64,
}
