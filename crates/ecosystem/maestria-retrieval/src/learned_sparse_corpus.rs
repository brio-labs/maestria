#[path = "learned_sparse_corpus_validation.rs"]
mod validation;
pub use validation::LearnedSparseCorpusError;

use maestria_domain::ContentHash;
use serde::{Deserialize, Serialize};

use crate::learned_sparse_benchmark::{
    LearnedSparseAcceptedSpan, LearnedSparseBenchmarkBudget, LearnedSparseDataFidelity,
    LearnedSparseDataSplit, LearnedSparseQueryClass,
};

pub const LEARNED_SPARSE_TASK_CORPUS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseTaskCorpus {
    pub schema_version: u32,
    pub corpus_id: String,
    pub corpus_revision: String,
    pub judgment_set_id: String,
    pub judgment_set_hash: ContentHash,
    pub source_input_hash: ContentHash,
    pub evaluation_date: String,
    pub source_inputs: Vec<LearnedSparseSourceInput>,
    pub judgment_guidance: LearnedSparseJudgmentGuidance,
    pub cases: Vec<LearnedSparseTaskCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseSourceInput {
    pub source_id: String,
    pub path: String,
    pub content_hash: ContentHash,
    pub role: LearnedSparseSourceRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseSourceRole {
    TaskDefinition,
    EvidenceSource,
    SecurityFixture,
    JudgmentGuidance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseJudgmentGuidance {
    pub relevance_scale: LearnedSparseRelevanceScale,
    pub independent_judges: u8,
    pub adjudication: LearnedSparseAdjudicationRule,
    pub citation_policy: LearnedSparseCitationPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseRelevanceScale {
    NotRelevantRelevantHighlyRelevant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseAdjudicationRule {
    IndependentThenThirdJudge,
    IndependentThenTaskOwner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseCitationPolicy {
    ExactAcceptedSpansRequired,
    AbstentionRequiresNoCitation,
    StatusOnlyWhenNoCitation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseTaskCase {
    pub case_id: String,
    pub task_id: String,
    pub class: LearnedSparseQueryClass,
    pub query: String,
    pub language: LearnedSparseQueryLanguage,
    pub split: LearnedSparseDataSplit,
    pub fidelity: LearnedSparseDataFidelity,
    pub tags: Vec<LearnedSparseCaseTag>,
    pub source_ids: Vec<String>,
    pub freshness: LearnedSparseFreshnessExpectation,
    pub security: Vec<LearnedSparseSecurityExpectation>,
    pub expected: LearnedSparseTaskExpectation,
    pub budget: LearnedSparseBenchmarkBudget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseQueryLanguage {
    English,
    Multilingual,
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LearnedSparseCaseTag {
    ExactIdentifier,
    Path,
    Filename,
    Symbol,
    Phrase,
    DuplicateSource,
    SupersededVersion,
    RareLiteral,
    Synonym,
    Paraphrase,
    VocabularyExpansion,
    DomainTerminology,
    MultiTerm,
    ShortQuery,
    LongQuery,
    AmbiguousQuery,
    NoEvidence,
    CorrectAbstention,
    Lifecycle,
    Migration,
    Activation,
    Deletion,
    Rollback,
    Privacy,
    Adversarial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseFreshnessExpectation {
    CurrentVersion,
    SupersededVersionsExcluded,
    StaleGenerationRejected,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseSecurityExpectation {
    ScopeFiltered,
    AclFiltered,
    SensitiveFiltered,
    Quarantined,
    SecretRejected,
    PromptInjectionRejected,
    PoisoningRejected,
    PrivacyDenied,
    ProviderUnavailable,
    IncompatibleIdentityRejected,
    StaleGenerationRejected,
    DeletionExcluded,
    PartialRebuildExcluded,
    RollbackRestoresBaseline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseTaskExpectation {
    Evidence {
        judgments: Vec<LearnedSparseEvidenceJudgment>,
        evidence_chain: Vec<String>,
        minimum_source_diversity: u32,
        citation: LearnedSparseCitationExpectation,
    },
    Abstain {
        reason: LearnedSparseAbstentionReason,
    },
    UnsupportedCapability {
        capability: String,
    },
    Conflict {
        source_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseEvidenceJudgment {
    pub source_id: String,
    pub grade: LearnedSparseRelevanceGrade,
    pub accepted_spans: Vec<LearnedSparseAcceptedSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseRelevanceGrade {
    NotRelevant,
    Relevant,
    HighlyRelevant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseCitationExpectation {
    Required,
    NotRequired,
    StatusOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseAbstentionReason {
    NoEvidence,
    SecurityPolicy,
    PrivacyPolicy,
    UnsupportedCapability,
    StaleGeneration,
    ProviderUnavailable,
}
