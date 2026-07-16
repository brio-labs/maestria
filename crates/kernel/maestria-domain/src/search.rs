use serde::{Deserialize, Serialize};
use std::fmt;

use crate::ids::*;
#[path = "search_outcome.rs"]
mod search_outcome;
pub use search_outcome::*;
#[path = "search_source.rs"]
mod search_source;
#[path = "search_trace.rs"]
mod search_trace;
pub use search_source::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchCompatibilityError {
    ModelFingerprintMismatch {
        expected: RetrievalModelFingerprint,
        found: RetrievalModelFingerprint,
    },
    IndexGenerationMismatch {
        expected: IndexGenerationId,
        found: IndexGenerationId,
    },
    InvalidBudget(&'static str),
    InvalidContentHash(&'static str),
    InvalidFingerprint(&'static str),
    InvalidSourceSpan(&'static str),
    InvalidCoverage(&'static str),
    InvalidModalitySet(&'static str),
    TracePlanMismatch(&'static str),
}

impl fmt::Display for SearchCompatibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelFingerprintMismatch { expected, found } => write!(
                f,
                "Incompatible retrieval model fingerprint: expected {}, found {}",
                expected.as_str(),
                found.as_str()
            ),
            Self::IndexGenerationMismatch { expected, found } => write!(
                f,
                "Incompatible index generation: expected {}, found {}",
                expected.value(),
                found.value()
            ),
            Self::InvalidBudget(msg) => write!(f, "Invalid budget: {}", msg),
            Self::InvalidContentHash(msg) => write!(f, "Invalid content hash: {}", msg),
            Self::InvalidFingerprint(msg) => {
                write!(f, "Invalid retrieval model fingerprint: {}", msg)
            }
            Self::InvalidSourceSpan(msg) => write!(f, "Invalid evidence span: {}", msg),
            Self::InvalidCoverage(msg) => write!(f, "Invalid evidence coverage: {}", msg),
            Self::InvalidModalitySet(msg) => write!(f, "Invalid modality set: {}", msg),
            Self::TracePlanMismatch(msg) => write!(f, "Search trace does not match plan: {}", msg),
        }
    }
}

impl std::error::Error for SearchCompatibilityError {}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RetrievalModelFingerprint(String);

impl TryFrom<String> for RetrievalModelFingerprint {
    type Error = SearchCompatibilityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        RetrievalModelFingerprint::new(value)
    }
}

impl RetrievalModelFingerprint {
    pub fn new(value: String) -> Result<Self, SearchCompatibilityError> {
        if value.trim().is_empty() {
            return Err(SearchCompatibilityError::InvalidFingerprint(
                "value must not be empty",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ContentHash(String);

impl TryFrom<String> for ContentHash {
    type Error = SearchCompatibilityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        ContentHash::new(value)
    }
}

impl ContentHash {
    pub fn new(hash: String) -> Result<Self, SearchCompatibilityError> {
        let valid_digest = hash.strip_prefix("sha256:").is_some_and(|digest| {
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        if !valid_digest {
            return Err(SearchCompatibilityError::InvalidContentHash(
                "Must be sha256: followed by 64 hexadecimal characters",
            ));
        }
        Ok(Self(hash))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ArtifactVersionDto")]
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

#[derive(Deserialize)]
struct ArtifactVersionDto {
    id: ArtifactVersionId,
    artifact_id: ArtifactId,
    content_hash: ContentHash,
}

impl TryFrom<ArtifactVersionDto> for ArtifactVersion {
    type Error = SearchCompatibilityError;

    fn try_from(dto: ArtifactVersionDto) -> Result<Self, Self::Error> {
        Ok(ArtifactVersion::new(
            dto.id,
            dto.artifact_id,
            dto.content_hash,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchIntent {
    ExactLookup,
    FactualLocal,
    SemanticDiscovery,
    CompositionalConstraints,
    MultiHop,
    CorpusSynthesis,
    RepositoryCode,
    VisualDocument,
    TemporalMemory,
    CurrentWeb,
    ContradictionAudit,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
#[serde(try_from = "ModalitySetDto")]
pub struct ModalitySet {
    values: Vec<Modality>,
}

#[derive(Deserialize)]
struct ModalitySetDto {
    values: Vec<Modality>,
}

impl TryFrom<ModalitySetDto> for ModalitySet {
    type Error = SearchCompatibilityError;

    fn try_from(dto: ModalitySetDto) -> Result<Self, Self::Error> {
        let mut values = dto.values;
        values.sort();
        values.dedup();
        Ok(Self { values })
    }
}

impl ModalitySet {
    pub fn new(values: Vec<Modality>) -> Self {
        let mut values = values;
        values.sort();
        values.dedup();
        Self { values }
    }

    pub fn values(&self) -> &[Modality] {
        &self.values
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchStage {
    InitialRetrieval,
    Reranking,
    Filtering,
    Synthesis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SearchBudgetDto")]
pub struct SearchBudget {
    max_tokens: u32,
    max_latency_ms: u32,
}

#[derive(Deserialize)]
struct SearchBudgetDto {
    max_tokens: u32,
    max_latency_ms: u32,
}

impl TryFrom<SearchBudgetDto> for SearchBudget {
    type Error = SearchCompatibilityError;

    fn try_from(dto: SearchBudgetDto) -> Result<Self, Self::Error> {
        SearchBudget::new(dto.max_tokens, dto.max_latency_ms)
    }
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
    #[serde(default)]
    pub required_claims: Vec<String>,
    #[serde(default)]
    pub required_subquestions: Vec<String>,
    #[serde(default)]
    pub minimum_sources: usize,
    #[serde(default)]
    pub minimum_documents: usize,
    #[serde(default)]
    pub minimum_sections: usize,
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
    pub fingerprint: RetrievalModelFingerprint,
}
