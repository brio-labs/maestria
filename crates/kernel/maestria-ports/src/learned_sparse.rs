use maestria_domain::{
    ChunkId, ContentHash, CorpusSnapshotId, IndexGenerationId, IndexLifecycle, RepresentationName,
    SparseNamespace,
};
use serde::{Deserialize, Serialize};

use crate::{BoundedSearch, PortError, ProviderDisclosure};

pub const SPARSE_REPRESENTATION_V1: &str = "sparse_text_v1";
pub const DEFAULT_MAX_SPARSE_TERMS: usize = 4_096;
pub const DEFAULT_MAX_CONTRIBUTIONS: usize = 16;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SparseFingerprint {
    pub provider: String,
    pub model: String,
    pub revision: String,
    pub artifact_hash: ContentHash,
    pub tokenizer_hash: ContentHash,
    pub vocabulary_hash: ContentHash,
    pub vocabulary_size: u32,
    pub term_namespace: String,
    pub query_template_hash: ContentHash,
    pub document_template_hash: ContentHash,
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
            ("preprocessing version", self.preprocessing_version.as_str()),
            ("weighting version", self.weighting_version.as_str()),
            ("quantization", self.quantization.as_str()),
        ];
        if let Some((label, _)) = required.iter().find(|(_, value)| value.trim().is_empty()) {
            return Err(PortError::InvalidInputContext {
                context: "sparse fingerprint field is empty",
                source: label.to_string(),
            });
        }
        if self.vocabulary_size == 0 {
            return Err(PortError::InvalidInputContext {
                context: "invalid sparse vocabulary size",
                source: "vocabulary size must be positive".to_string(),
            });
        }
        if self.max_terms == 0 || self.max_terms > self.vocabulary_size {
            return Err(PortError::InvalidInputContext {
                context: "invalid sparse max term budget",
                source: "max_terms must be within the vocabulary".to_string(),
            });
        }
        if !self.pruning_threshold.is_finite() || self.pruning_threshold < 0.0 {
            return Err(PortError::InvalidInputContext {
                context: "invalid sparse pruning threshold",
                source: "threshold must be finite and non-negative".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SparseIdentity {
    pub generation_id: IndexGenerationId,
    pub corpus_snapshot: CorpusSnapshotId,
    pub representation: RepresentationName,
    pub namespace: SparseNamespace,
    pub fingerprint: SparseFingerprint,
}

impl SparseIdentity {
    pub fn validate(&self) -> Result<(), PortError> {
        if self.representation.0 != SPARSE_REPRESENTATION_V1 {
            return Err(PortError::InvalidInputContext {
                context: "invalid sparse representation",
                source: self.representation.0.clone(),
            });
        }
        self.namespace.validate().map_err(|error| {
            PortError::invalid_input("invalid sparse namespace", error.to_string())
        })?;
        if self.namespace.projection() != self.representation.0 {
            return Err(PortError::InvalidInputContext {
                context: "sparse namespace projection mismatch",
                source: self.namespace.projection().to_string(),
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
            return Err(PortError::InvalidInputContext {
                context: "invalid sparse term weight",
                source: "weight must be finite and positive".to_string(),
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
            return Err(PortError::InvalidInputContext {
                context: "sparse vector is empty",
                source: "at least one term is required".to_string(),
            });
        }
        let max_terms = usize::try_from(identity.fingerprint.max_terms).map_err(|_| {
            PortError::InvalidInputContext {
                context: "sparse max_terms exceeds platform range",
                source: "max_terms does not fit this platform".to_string(),
            }
        })?;
        if terms.len() > max_terms || terms.len() > DEFAULT_MAX_SPARSE_TERMS {
            return Err(PortError::InvalidInputContext {
                context: "sparse vector exceeds term budget",
                source: "term count exceeds max_terms or default limit".to_string(),
            });
        }
        terms.sort_by_key(|term| term.term_id);
        for window in terms.windows(2) {
            if window[0].term_id == window[1].term_id {
                return Err(PortError::InvalidInputContext {
                    context: "sparse vector contains duplicate term identifiers",
                    source: "term identifiers must be unique".to_string(),
                });
            }
        }
        if terms
            .iter()
            .any(|term| term.term_id >= identity.fingerprint.vocabulary_size)
        {
            return Err(PortError::InvalidInputContext {
                context: "sparse term identifier is outside vocabulary",
                source: "term identifier must be less than vocabulary size".to_string(),
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
    pub execution_budget: maestria_domain::SearchExecutionBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SparseTermContribution {
    pub term_id: u32,
    pub contribution_micros: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SparseSearchHit {
    pub chunk_id: ChunkId,
    pub score_micros: u32,
    pub contributions: Vec<SparseTermContribution>,
}

/// Two-pointer dot-product merge over sorted term weights shared by every
/// sparse scoring implementation so the contribution math cannot diverge.
pub fn dot_contributions(
    document: &[SparseTermWeight],
    query: &[SparseTermWeight],
) -> Vec<(u32, f64)> {
    let mut left = 0_usize;
    let mut right = 0_usize;
    let capacity = document.len().min(query.len()).min(16);
    let mut contributions = Vec::with_capacity(capacity);
    while left < document.len() && right < query.len() {
        let document_term = document[left];
        let query_term = query[right];
        match document_term.term_id().cmp(&query_term.term_id()) {
            std::cmp::Ordering::Less => left += 1,
            std::cmp::Ordering::Greater => right += 1,
            std::cmp::Ordering::Equal => {
                contributions.push((
                    document_term.term_id(),
                    f64::from(document_term.weight()) * f64::from(query_term.weight()),
                ));
                left += 1;
                right += 1;
            }
        }
    }
    contributions
}

/// Micros-scaled fixed-point clamp shared by every sparse scorer: non-finite
/// or non-positive values collapse to zero; otherwise the micros value is
/// rounded and clamped to the u32 range.
pub fn fixed_micros(value: f64) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    (value * 1_000_000.0).round().min(f64::from(u32::MAX)) as u32
}

/// Sort-by-score completion tail shared by every sparse scorer: score
/// descending, chunk id ascending, then result-budget metering and
/// complete/exhausted termination.
pub fn finish_sparse_search(
    mut meter: crate::execution::Meter,
    mut hits: Vec<SparseSearchHit>,
    limit: usize,
    mut stopped: Option<maestria_domain::SearchExecutionResource>,
) -> BoundedSearch<SparseSearchHit> {
    hits.sort_by(|left, right| {
        right
            .score_micros
            .cmp(&left.score_micros)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    hits.truncate(limit);
    for _ in &hits {
        if let Some(resource) = meter.result() {
            stopped = Some(resource);
            break;
        }
    }
    match stopped {
        Some(resource) => meter.exhausted(hits, resource),
        None => meter.complete(hits),
    }
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

    /// Encodes many texts in one provider call.
    ///
    /// The default implementation loops [`Self::encode`]; loopback providers
    /// with a batch endpoint override it to amortize the per-request cost.
    fn encode_batch(
        &self,
        texts: &[String],
        kind: SparseInputKind,
        identity: SparseIdentity,
    ) -> Result<Vec<SparseVector>, PortError> {
        texts
            .iter()
            .map(|text| self.encode(text, kind, identity.clone()))
            .collect()
    }
}
pub trait LearnedSparseIndex: Send + Sync {
    fn identity(&self) -> Option<SparseIdentity>;

    fn index_documents(&self, documents: Vec<SparseDocument>) -> Result<(), PortError>;

    fn search(&self, query: SparseSearchQuery)
    -> Result<BoundedSearch<SparseSearchHit>, PortError>;

    fn search_filtered(
        &self,
        query: SparseSearchQuery,
        filter: &dyn Fn(ChunkId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<SparseSearchHit>, PortError> {
        let _ = (query, filter);
        Err(PortError::InternalContext {
            context: "sparse filtered search is unsupported",
            source: "implementation must provide governed retrieval filtering".to_string(),
        })
    }

    fn delete_chunks(&self, chunk_ids: &[ChunkId]) -> Result<(), PortError>;

    fn clear(&self) -> Result<(), PortError>;

    fn rebuild(&self, documents: Vec<SparseDocument>) -> Result<(), PortError> {
        self.clear()?;
        self.index_documents(documents)
    }
}
/// Durable projection lifecycle mirroring the shared index-generation registry.
///
/// The registry remains the lifecycle owner. Adapters persist and validate the
/// caller-provided transition so a partially built or retired projection cannot
/// become searchable by accident.
pub trait LearnedSparseProjectionLifecycle: Send + Sync {
    fn lifecycle(&self) -> Result<IndexLifecycle, PortError>;

    fn transition(&self, expected: IndexLifecycle, next: IndexLifecycle) -> Result<(), PortError>;

    fn collect(&self) -> Result<(), PortError>;
}
