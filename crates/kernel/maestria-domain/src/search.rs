use serde::{Deserialize, Serialize};
use std::fmt;

use crate::ContentRange;
use crate::ids::*;

/// Module-level invariants:
/// - `ContentHash` always starts with a recognized algorithm prefix (e.g., `sha256:`).
/// - `ArtifactVersion` holds a globally unique `ArtifactVersionId` and matches its `ContentHash`.
/// - `SearchBudget` values are strictly positive and constructors reject invalid combinations via `SearchCompatibilityError`.
/// - Search generation footprints must match across plan and outcome; a mismatch yields `SearchCompatibilityError`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchCompatibilityError {
    ModelFingerprintMismatch {
        expected: RetrievalModelFingerprintId,
        found: RetrievalModelFingerprintId,
    },
    IndexGenerationMismatch {
        expected: IndexGenerationId,
        found: IndexGenerationId,
    },
    InvalidBudget(&'static str),
    InvalidContentHash(&'static str),
}

impl fmt::Display for SearchCompatibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelFingerprintMismatch { expected, found } => write!(
                f,
                "Incompatible retrieval model fingerprint: expected {}, found {}",
                expected.value(),
                found.value()
            ),
            Self::IndexGenerationMismatch { expected, found } => write!(
                f,
                "Incompatible index generation: expected {}, found {}",
                expected.value(),
                found.value()
            ),
            Self::InvalidBudget(msg) => write!(f, "Invalid budget: {}", msg),
            Self::InvalidContentHash(msg) => write!(f, "Invalid content hash: {}", msg),
        }
    }
}

impl std::error::Error for SearchCompatibilityError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn new(hash: String) -> Result<Self, SearchCompatibilityError> {
        if !hash.starts_with("sha256:") {
            return Err(SearchCompatibilityError::InvalidContentHash(
                "Must start with sha256:",
            ));
        }
        Ok(Self(hash))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    id: ArtifactVersionId,
    artifact_id: ArtifactId,
    content_hash: ContentHash,
}

impl ArtifactVersion {
    pub fn new(id: ArtifactVersionId, artifact_id: ArtifactId, content_hash: ContentHash) -> Self {
        Self {
            id,
            artifact_id,
            content_hash,
        }
    }

    pub fn id(&self) -> ArtifactVersionId {
        self.id
    }

    pub fn artifact_id(&self) -> ArtifactId {
        self.artifact_id
    }

    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructureNodeType {
    Document,
    Section,
    Paragraph,
    List,
    ListItem,
    Table,
    Figure,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructureNode {
    pub id: StructureNodeId,
    pub parent_id: Option<StructureNodeId>,
    pub sibling_id: Option<StructureNodeId>,
    pub node_type: StructureNodeType,
    pub source_range: ContentRange,
    pub page: Option<u32>,
    pub section_path: Vec<String>,
    pub parser_generation: String,
    pub schema_generation: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceLocation {
    File {
        path: String,
        start_line: u32,
        end_line: u32,
    },
    Page {
        page_start: u32,
        page_end: u32,
    },
    Region {
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Symbol {
        path: String,
        qualified_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceSpan {
    pub node_id: Option<StructureNodeId>,
    pub location: SourceLocation,
    pub range: ContentRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchIntent {
    Navigational,
    Informational,
    Transactional,
    Exploratory,
    FactVerification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CorpusScope {
    Global,
    Restricted(Vec<ScopeId>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreshnessRequirement {
    Any,
    Realtime,
    MaximumAgeDays(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modality {
    Text,
    Image,
    Code,
    Pdf,
    Table,
    Web,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalitySet {
    pub values: Vec<Modality>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchStage {
    InitialRetrieval,
    Reranking,
    Filtering,
    Synthesis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchBudget {
    max_tokens: u32,
    max_latency_ms: u32,
}

impl SearchBudget {
    pub fn new(max_tokens: u32, max_latency_ms: u32) -> Result<Self, SearchCompatibilityError> {
        if max_tokens == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_tokens must be greater than 0",
            ));
        }
        if max_latency_ms == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_latency_ms must be greater than 0",
            ));
        }
        Ok(Self {
            max_tokens,
            max_latency_ms,
        })
    }

    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    pub fn max_latency_ms(&self) -> u32 {
        self.max_latency_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopConditions {
    pub max_results: u32,
    pub min_score_threshold: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRequirements {
    pub require_primary_sources: bool,
    pub minimum_corroboration: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPlan {
    pub query_id: QueryId,
    pub original_query: String,
    pub intent: SearchIntent,
    pub scope: CorpusScope,
    pub corpus_snapshot: CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub freshness: FreshnessRequirement,
    pub modalities: ModalitySet,
    pub stages: Vec<SearchStage>,
    pub budgets: SearchBudget,
    pub stop_conditions: StopConditions,
    pub evidence_requirements: EvidenceRequirements,
    pub fingerprint: RetrievalModelFingerprintId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalScoreSet {
    pub bm25: u32,
    pub semantic_similarity: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustLabel {
    Verified,
    Unverified,
    Disputed,
    Deprecated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreshnessStatus {
    UpToDate,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetrievalReason {
    ExactMatch,
    SemanticSimilarity,
    CitationLink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceCandidate {
    pub artifact_version: ArtifactVersion,
    pub source_span: EvidenceSpan,
    pub retrieval_score: RetrievalScoreSet,
    pub trust_label: TrustLabel,
    pub freshness: FreshnessStatus,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub reasons: Vec<RetrievalReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceCoverage {
    pub percent_covered: u8,
    pub gaps_identified: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictSet {
    pub id: ConflictSetId,
    pub candidates: Vec<EvidenceCandidate>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchStatus {
    Success,
    PartialResults,
    Timeout,
    ExhaustedBudget,
    DeniedByPolicy,
    QuarantinedForReview,
    Abstained,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOutcome {
    pub trace_id: SearchTraceId,
    pub fingerprint: RetrievalModelFingerprintId,
    pub index_generation: IndexGenerationId,
    pub status: SearchStatus,
    pub coverage: EvidenceCoverage,
    pub conflicts: Vec<ConflictSet>,
    pub candidates: Vec<EvidenceCandidate>,
}

impl SearchOutcome {
    pub fn verify_compatibility(&self, plan: &SearchPlan) -> Result<(), SearchCompatibilityError> {
        if self.fingerprint != plan.fingerprint {
            return Err(SearchCompatibilityError::ModelFingerprintMismatch {
                expected: plan.fingerprint,
                found: self.fingerprint,
            });
        }
        if self.index_generation != plan.index_generation {
            return Err(SearchCompatibilityError::IndexGenerationMismatch {
                expected: plan.index_generation,
                found: self.index_generation,
            });
        }
        Ok(())
    }
}
