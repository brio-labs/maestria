use serde::{Deserialize, Serialize};
use std::fmt;

use crate::ids::*;
#[path = "search_plan.rs"]
mod search_plan;
pub use search_plan::*;
#[path = "search_intent.rs"]
mod search_intent;
pub use search_intent::*;
#[path = "retrieval_score.rs"]
mod retrieval_score;
pub use retrieval_score::*;
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
    InvalidPlan(&'static str),
    InvalidScoreProvenance(&'static str),
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
            Self::InvalidPlan(msg) => write!(f, "Invalid search plan: {}", msg),
            Self::InvalidScoreProvenance(msg) => {
                write!(f, "Invalid retrieval score provenance: {}", msg)
            }
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
