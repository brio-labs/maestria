use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::SearchCompatibilityError;
use crate::ids::{CorpusSnapshotId, IndexGenerationId, QueryId, ScopeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

impl SearchIntent {
    /// Classifies a query using deterministic lexical signals only.
    pub fn classify(query: &str) -> Self {
        let query = query.trim().to_ascii_lowercase();
        let has = |terms: &[&str]| {
            terms.iter().any(|term| {
                let is_token = term
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_');
                if is_token {
                    query
                        .split(|character: char| {
                            !character.is_ascii_alphanumeric() && character != '_'
                        })
                        .any(|token| token == *term || token.starts_with(term))
                } else {
                    query.contains(term)
                }
            })
        };
        if query.is_empty()
            || (query.starts_with('"') && query.ends_with('"'))
            || has(&["id:", "::", ".rs", "cargo.toml", "path:"])
        {
            Self::ExactLookup
        } else if has(&["contradict", "conflict", "disagree", "counterevidence"]) {
            Self::ContradictionAudit
        } else if has(&["latest", "today", "current", "web", "news", "http"]) {
            Self::CurrentWeb
        } else if has(&["table", "chart", "figure", "image", "visual", "pdf"]) {
            Self::VisualDocument
        } else if has(&[
            "rust",
            "cargo",
            "function",
            "struct",
            "trait",
            "module",
            "repository",
        ]) {
            Self::RepositoryCode
        } else if has(&["when", "before", "after", "history", "previous", "last"]) {
            Self::TemporalMemory
        } else if has(&["how does", "relationship", "connected", "multi-hop"]) {
            Self::MultiHop
        } else if has(&[
            "summarize",
            "summary",
            "overview",
            "across",
            "compare",
            "synthesis",
        ]) {
            Self::CorpusSynthesis
        } else if has(&["must", "without", "requires", "constraint"])
            || (query.contains(" and ") && query.contains(" where "))
        {
            Self::CompositionalConstraints
        } else if has(&["similar", "related", "discover", "explore"]) {
            Self::SemanticDiscovery
        } else {
            Self::FactualLocal
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SearchStage {
    InitialRetrieval,
    Reranking,
    Filtering,
    Synthesis,
}

fn default_one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SearchBudgetDto")]
pub struct SearchBudget {
    max_tokens: u32,
    max_latency_ms: u32,
    max_queries: u32,
    max_stages: u32,
    max_web_requests: u32,
}

#[derive(Deserialize)]
struct SearchBudgetDto {
    max_tokens: u32,
    max_latency_ms: u32,
    #[serde(default = "default_one")]
    max_queries: u32,
    #[serde(default = "default_one")]
    max_stages: u32,
    #[serde(default)]
    max_web_requests: u32,
}

impl TryFrom<SearchBudgetDto> for SearchBudget {
    type Error = SearchCompatibilityError;

    fn try_from(dto: SearchBudgetDto) -> Result<Self, Self::Error> {
        Self::with_limits(
            dto.max_tokens,
            dto.max_latency_ms,
            dto.max_queries,
            dto.max_stages,
            dto.max_web_requests,
        )
    }
}

impl SearchBudget {
    pub fn new(max_tokens: u32, max_latency_ms: u32) -> Result<Self, SearchCompatibilityError> {
        Self::with_limits(max_tokens, max_latency_ms, 1, 1, 0)
    }

    pub fn with_limits(
        max_tokens: u32,
        max_latency_ms: u32,
        max_queries: u32,
        max_stages: u32,
        max_web_requests: u32,
    ) -> Result<Self, SearchCompatibilityError> {
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
        if max_queries == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_queries must be greater than 0",
            ));
        }
        if max_stages == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_stages must be greater than 0",
            ));
        }
        Ok(Self {
            max_tokens,
            max_latency_ms,
            max_queries,
            max_stages,
            max_web_requests,
        })
    }

    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    pub fn max_latency_ms(&self) -> u32 {
        self.max_latency_ms
    }

    pub fn max_queries(&self) -> u32 {
        self.max_queries
    }

    pub fn max_stages(&self) -> u32 {
        self.max_stages
    }

    pub fn max_web_requests(&self) -> u32 {
        self.max_web_requests
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
    pub fingerprint: super::RetrievalModelFingerprint,
}

impl SearchPlan {
    /// Validates schema invariants before policy or runtime evaluation.
    pub fn validate_schema(&self) -> Result<(), SearchCompatibilityError> {
        if self.original_query.trim().is_empty() {
            return Err(SearchCompatibilityError::InvalidPlan(
                "original_query must not be empty",
            ));
        }
        let query_tokens = self.original_query.split_whitespace().count().max(1);
        if query_tokens > self.budgets.max_tokens() as usize {
            return Err(SearchCompatibilityError::InvalidPlan(
                "original_query exceeds the token budget",
            ));
        }
        if self.modalities.values().is_empty() {
            return Err(SearchCompatibilityError::InvalidPlan(
                "at least one modality is required",
            ));
        }
        if self.stages.is_empty() {
            return Err(SearchCompatibilityError::InvalidPlan(
                "at least one search stage is required",
            ));
        }
        if self.stages[0] != SearchStage::InitialRetrieval {
            return Err(SearchCompatibilityError::InvalidPlan(
                "initial retrieval must be the first stage",
            ));
        }
        let unique_stages = self.stages.iter().collect::<BTreeSet<_>>();
        if unique_stages.len() != self.stages.len() {
            return Err(SearchCompatibilityError::InvalidPlan(
                "search stages must not repeat",
            ));
        }
        if self.stages.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(SearchCompatibilityError::InvalidPlan(
                "search stages must use canonical execution order",
            ));
        }
        if self.stages.len() > self.budgets.max_stages() as usize {
            return Err(SearchCompatibilityError::InvalidPlan(
                "search stages exceed the stage budget",
            ));
        }
        if self.stop_conditions.max_results == 0 {
            return Err(SearchCompatibilityError::InvalidPlan(
                "max_results must be greater than 0",
            ));
        }
        if self.stop_conditions.min_score_threshold > 10_000 {
            return Err(SearchCompatibilityError::InvalidPlan(
                "min_score_threshold must be between 0 and 10000",
            ));
        }
        if let CorpusScope::Restricted(scopes) = &self.scope {
            if scopes.is_empty() {
                return Err(SearchCompatibilityError::InvalidPlan(
                    "restricted scope must contain at least one scope",
                ));
            }
            let unique_scopes = scopes.iter().collect::<BTreeSet<_>>();
            if unique_scopes.len() != scopes.len() {
                return Err(SearchCompatibilityError::InvalidPlan(
                    "restricted scope identifiers must not repeat",
                ));
            }
        }
        if matches!(self.freshness, FreshnessRequirement::MaximumAgeDays(0)) {
            return Err(SearchCompatibilityError::InvalidPlan(
                "maximum freshness age must be greater than 0 days",
            ));
        }
        if (self.intent == SearchIntent::CurrentWeb
            || self.modalities.values().contains(&Modality::Web))
            && self.budgets.max_web_requests() == 0
        {
            return Err(SearchCompatibilityError::InvalidPlan(
                "web plans require a positive web request budget",
            ));
        }
        if self.evidence_requirements.minimum_corroboration == 0 {
            return Err(SearchCompatibilityError::InvalidPlan(
                "minimum corroboration must be greater than 0",
            ));
        }
        if self
            .evidence_requirements
            .required_claims
            .iter()
            .chain(self.evidence_requirements.required_subquestions.iter())
            .any(|value| value.trim().is_empty())
        {
            return Err(SearchCompatibilityError::InvalidPlan(
                "required claims and subquestions must not be empty",
            ));
        }
        Ok(())
    }
}
