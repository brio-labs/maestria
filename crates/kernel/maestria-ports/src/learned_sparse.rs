use maestria_domain::{
    ChunkId, ContentHash, CorpusSnapshotId, IndexGenerationId, RepresentationName,
};

use crate::{PortError, ProviderDisclosure};

pub const SPARSE_REPRESENTATION_V1: &str = "sparse_text_v1";
pub const DEFAULT_MAX_SPARSE_TERMS: usize = 4_096;
pub const DEFAULT_MAX_CONTRIBUTIONS: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct SparseFingerprint {
    pub provider: String,
    pub model: String,
    pub revision: String,
    pub artifact_hash: ContentHash,
    pub tokenizer_hash: ContentHash,
    pub vocabulary_hash: ContentHash,
    pub vocabulary_size: u32,
    pub term_namespace: String,
    pub query_template_hash: String,
    pub document_template_hash: String,
    pub preprocessing_version: String,
    pub weighting_version: String,
    pub quantization: String,
    pub pruning_threshold: f32,
    pub max_terms: u32,
}

impl SparseFingerprint {
    pub fn validate(&self) -> Result<(), PortError> {
        let required = [
            ("provider", self.provider.as_str()),
            ("model", self.model.as_str()),
            ("revision", self.revision.as_str()),
            ("term namespace", self.term_namespace.as_str()),
            ("query template hash", self.query_template_hash.as_str()),
            (
                "document template hash",
                self.document_template_hash.as_str(),
            ),
            ("preprocessing version", self.preprocessing_version.as_str()),
            ("weighting version", self.weighting_version.as_str()),
            ("quantization", self.quantization.as_str()),
        ];
        if let Some((label, _)) = required.iter().find(|(_, value)| value.trim().is_empty()) {
            return Err(PortError::InvalidInput {
                message: format!("sparse fingerprint {label} must not be empty"),
            });
        }
        if self.vocabulary_size == 0 {
            return Err(PortError::InvalidInput {
                message: "sparse fingerprint vocabulary size must be positive".to_string(),
            });
        }
        if self.max_terms == 0 || self.max_terms > self.vocabulary_size {
            return Err(PortError::InvalidInput {
                message: "sparse fingerprint max_terms must be within the vocabulary".to_string(),
            });
        }
        if !self.pruning_threshold.is_finite() || self.pruning_threshold < 0.0 {
            return Err(PortError::InvalidInput {
                message: "sparse fingerprint pruning threshold must be finite and non-negative"
                    .to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SparseIdentity {
    pub generation_id: IndexGenerationId,
    pub corpus_snapshot: CorpusSnapshotId,
    pub representation: RepresentationName,
    pub fingerprint: SparseFingerprint,
}

impl SparseIdentity {
    pub fn validate(&self) -> Result<(), PortError> {
        if self.representation.0 != SPARSE_REPRESENTATION_V1 {
            return Err(PortError::InvalidInput {
                message: format!(
                    "sparse representation must be {SPARSE_REPRESENTATION_V1}, found {}",
                    self.representation.0
                ),
            });
        }
        self.fingerprint.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SparseTermWeight {
    term_id: u32,
    weight: f32,
}

impl SparseTermWeight {
    pub fn new(term_id: u32, weight: f32) -> Result<Self, PortError> {
        if !weight.is_finite() || weight <= 0.0 {
            return Err(PortError::InvalidInput {
                message: "sparse term weight must be finite and positive".to_string(),
            });
        }
        Ok(Self { term_id, weight })
    }

    pub fn term_id(self) -> u32 {
        self.term_id
    }

    pub fn weight(self) -> f32 {
        self.weight
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SparseVector {
    identity: SparseIdentity,
    terms: Vec<SparseTermWeight>,
}

impl SparseVector {
    pub fn new(
        identity: SparseIdentity,
        mut terms: Vec<SparseTermWeight>,
    ) -> Result<Self, PortError> {
        identity.validate()?;
        if terms.is_empty() {
            return Err(PortError::InvalidInput {
                message: "sparse vector must contain at least one term".to_string(),
            });
        }
        let max_terms = usize::try_from(identity.fingerprint.max_terms).map_err(|_| {
            PortError::InvalidInput {
                message: "sparse max_terms does not fit this platform".to_string(),
            }
        })?;
        if terms.len() > max_terms || terms.len() > DEFAULT_MAX_SPARSE_TERMS {
            return Err(PortError::InvalidInput {
                message: "sparse vector exceeds its term budget".to_string(),
            });
        }
        terms.sort_by_key(|term| term.term_id);
        for window in terms.windows(2) {
            if window[0].term_id == window[1].term_id {
                return Err(PortError::InvalidInput {
                    message: "sparse vector contains duplicate term identifiers".to_string(),
                });
            }
        }
        if terms
            .iter()
            .any(|term| term.term_id >= identity.fingerprint.vocabulary_size)
        {
            return Err(PortError::InvalidInput {
                message: "sparse term identifier is outside the declared vocabulary".to_string(),
            });
        }
        Ok(Self { identity, terms })
    }

    pub fn identity(&self) -> &SparseIdentity {
        &self.identity
    }

    pub fn terms(&self) -> &[SparseTermWeight] {
        &self.terms
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SparseDocument {
    pub chunk_id: ChunkId,
    pub content_hash: ContentHash,
    pub vector: SparseVector,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SparseSearchQuery {
    pub vector: SparseVector,
    pub limit: u32,
    pub max_contributions: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SparseTermContribution {
    pub term_id: u32,
    pub contribution_micros: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseSearchHit {
    pub chunk_id: ChunkId,
    pub score_micros: u32,
    pub contributions: Vec<SparseTermContribution>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SparseInputKind {
    Document,
    Query,
}

pub trait LearnedSparseProvider: Send + Sync {
    fn disclosure(&self) -> Option<ProviderDisclosure>;
    fn identity(&self) -> Option<SparseIdentity>;
    fn encode(
        &self,
        text: &str,
        kind: SparseInputKind,
        identity: SparseIdentity,
    ) -> Result<SparseVector, PortError>;
}

pub trait LearnedSparseIndex: Send + Sync {
    fn index_documents(&self, documents: Vec<SparseDocument>) -> Result<(), PortError>;

    fn search(&self, query: SparseSearchQuery) -> Result<Vec<SparseSearchHit>, PortError>;

    fn search_filtered(
        &self,
        query: SparseSearchQuery,
        filter: &dyn Fn(ChunkId) -> bool,
    ) -> Result<Vec<SparseSearchHit>, PortError> {
        let _ = (query, filter);
        Err(PortError::Internal {
            message: "sparse search_filtered is required for governed retrieval".to_string(),
        })
    }

    fn delete_chunks(&self, chunk_ids: &[ChunkId]) -> Result<(), PortError>;

    fn clear(&self) -> Result<(), PortError>;

    fn rebuild(&self, documents: Vec<SparseDocument>) -> Result<(), PortError> {
        self.clear()?;
        self.index_documents(documents)
    }
}
