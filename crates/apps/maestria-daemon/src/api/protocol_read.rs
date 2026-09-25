use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub instance_root: String,
    pub event_count: usize,
    pub task_count: usize,
    pub socket_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub query_id: u64,
    pub trace_id: u64,
    pub status: String,
    pub fingerprint: String,
    pub index_generation: u64,
    pub evidence: Vec<SearchEvidenceResponse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_results: Vec<SearchPathResultResponse>,
    pub coverage: CoverageResponse,
    pub conflict_count: usize,
}

/// A fresh, approved filename/path match. It deliberately contains no excerpt,
/// fabricated citation, or line number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPathResultResponse {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEvidenceResponse {
    pub evidence_id: u64,
    pub artifact_version: u64,
    pub source: String,
    pub range_start: usize,
    pub range_end: usize,
    pub score_schema_version: u16,
    pub scores: Vec<SearchScoreResponse>,
    pub trust: String,
    pub freshness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<SearchPassagePreviewResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPassagePreviewResponse {
    pub excerpt: String,
    pub truncated: bool,
    pub location: EvidenceSourceResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchScoreResponse {
    pub score_kind: String,
    pub raw_score: i64,
    pub raw_rank: SearchRawRankResponse,
    pub scale: SearchScoreScaleResponse,
    pub representation: String,
    pub fingerprint: String,
    pub fingerprint_components: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SearchRawRankResponse {
    Ranked { rank: u32 },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchScoreScaleResponse {
    Binary,
    Unbounded {
        name: String,
        higher_is_better: bool,
    },
    FixedPoint {
        name: String,
        denominator: u32,
        minimum: Option<i64>,
        maximum: Option<i64>,
        higher_is_better: bool,
    },
    RankDerived {
        name: String,
        higher_is_better: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageResponse {
    pub percent_covered: u8,
    pub gaps: Vec<String>,
    pub distinct_sources: usize,
    pub distinct_documents: usize,
    pub distinct_sections: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceResponse {
    pub evidence_id: u64,
    pub artifact_id: u64,
    pub artifact_title: String,
    pub artifact_content_hash: Option<String>,
    pub source: EvidenceSourceResponse,
    pub excerpt: String,
    pub observed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceSourceResponse {
    File {
        path: String,
        start_line: u32,
        end_line: u32,
        content_hash: String,
    },
    DocxParagraph {
        path: String,
        start_paragraph: u32,
        end_paragraph: u32,
        content_hash: String,
    },
    Pdf {
        snapshot_id: u64,
        page_start: u32,
        page_end: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    PdfRegion {
        snapshot_id: u64,
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Web {
        url: String,
        content_hash: String,
        snapshot_id: u64,
    },
    Command {
        harness_run: u64,
        stream: String,
        blob_id: u64,
    },
    Test {
        harness_run: u64,
        status: String,
        log_id: u64,
    },
    Diff {
        harness_run: u64,
        patch_blob_id: u64,
    },
    Validation {
        report_id: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResponse {
    pub tasks: Vec<TaskSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalStatusResponse {
    pub index_generation: u64,
    pub corpus_snapshot: u64,
    pub fingerprint: String,
    pub lanes: RetrievalLaneStatus,
    pub promotion_records: RetrievalPromotionRecords,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRootsStatusResponse {
    pub roots: Vec<SearchRootStatus>,
    pub approved_root_count: usize,
    pub roots_truncated: bool,
    pub ocr_needed_file_count: usize,
    pub supported_formats: Vec<String>,
    pub ignored_by_default: Vec<String>,
    pub privacy_exclusions: Vec<String>,
    pub privacy_exclusions_truncated: bool,
    pub excluded_sources: Vec<SearchExcludedSource>,
    pub excluded_sources_truncated: bool,
    pub exclusion_scan_truncated: bool,
    pub indexing: SearchIndexingStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRootStatus {
    pub path: String,
    pub path_truncated: bool,
    pub indexed_file_count: usize,
    pub indexed_sources: Vec<String>,
    pub indexed_sources_truncated: bool,
    pub indexed_source_paths_truncated: bool,
    pub excluded_file_count: usize,
    pub exclusions_by_reason: std::collections::BTreeMap<String, usize>,
    pub exclusion_scan_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchExcludedSource {
    pub root: String,
    pub path: String,
    pub path_truncated: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndexingStatus {
    pub scanning: bool,
    pub pending_file_count: usize,
    pub last_scan_unix_ms: Option<u64>,
    pub last_error: Option<String>,
    pub last_error_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalLaneStatus {
    /// "Shadow" | "Active"
    pub hybrid_state: String,
    /// PascalCase class names, e.g. "DomainTerminology"
    pub hybrid_served_classes: Vec<String>,
    pub hybrid_evaluation_id: Option<String>,
    pub hybrid_evaluation_date: Option<String>,
    pub hybrid_report_hash: Option<String>,
    /// "Disabled" | "Shadow" | "Active"
    pub learned_sparse_state: String,
    pub learned_sparse_model: Option<String>,
    pub dense_enabled: bool,
    pub dense_model: Option<String>,
    /// "Shadow" | "Active"
    pub repository_code_state: String,
    /// "Shadow" | "Active"
    pub visual_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalPromotionRecords {
    pub learned_sparse: Option<RetrievalPromotionRecordWire>,
    pub hybrid: Option<RetrievalPromotionRecordWire>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalPromotionRecordWire {
    pub evaluation_id: String,
    pub corpus_id: String,
    pub evaluation_date: String,
    pub report_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub task_id: u64,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub evidence_ids: Vec<u64>,
    pub validation_report_id: Option<u64>,
}
