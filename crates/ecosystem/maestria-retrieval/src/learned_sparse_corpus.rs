#[path = "learned_sparse_corpus_validation.rs"]
mod validation;
pub use validation::LearnedSparseCorpusError;

use std::collections::BTreeMap;

use maestria_domain::{ContentHash, CorpusSnapshotId, IndexGenerationId, SparseNamespace};
use serde::{Deserialize, Serialize};

use crate::learned_sparse_benchmark::{
    LearnedSparseAcceptedSpan, LearnedSparseBenchmarkBudget, LearnedSparseBenchmarkCase,
    LearnedSparseBenchmarkCorpus, LearnedSparseDataFidelity, LearnedSparseDataSplit,
    LearnedSparseEnvironment, LearnedSparseExpectedOutcome, LearnedSparseQueryClass,
    LearnedSparseRoute, LearnedSparseRouteConfiguration,
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

impl LearnedSparseTaskCorpus {
    /// Converts the frozen task corpus into a v2 benchmark corpus bound to a
    /// real instance's corpus snapshot, sparse generation, and namespace.
    ///
    /// Task expectations map onto benchmark outcomes: evidence judgments
    /// become accepted spans and chains, unsupported capabilities become
    /// `UnsupportedClaim`, and conflicts keep their source set. The result
    /// must pass the benchmark corpus validation, so a missing route
    /// configuration or identity fails here.
    pub fn to_benchmark_corpus(
        &self,
        environment: LearnedSparseEnvironment,
        route_configurations: BTreeMap<LearnedSparseRoute, LearnedSparseRouteConfiguration>,
        corpus_snapshot: CorpusSnapshotId,
        index_generation: IndexGenerationId,
        namespace: SparseNamespace,
    ) -> Result<LearnedSparseBenchmarkCorpus, LearnedSparseCorpusError> {
        self.validate()?;
        let cases = self
            .cases
            .iter()
            .map(|case| {
                let expected = match &case.expected {
                    LearnedSparseTaskExpectation::Evidence {
                        judgments,
                        evidence_chain,
                        minimum_source_diversity,
                        ..
                    } => {
                        let mut accepted_spans = Vec::new();
                        for judgment in judgments {
                            accepted_spans.extend(judgment.accepted_spans.iter().cloned());
                        }
                        LearnedSparseExpectedOutcome::Evidence {
                            accepted_spans,
                            evidence_chain: evidence_chain.clone(),
                            minimum_source_diversity: *minimum_source_diversity,
                        }
                    }
                    LearnedSparseTaskExpectation::Abstain { .. } => {
                        LearnedSparseExpectedOutcome::Abstain
                    }
                    LearnedSparseTaskExpectation::UnsupportedCapability { .. } => {
                        LearnedSparseExpectedOutcome::UnsupportedClaim
                    }
                    LearnedSparseTaskExpectation::Conflict { .. } => {
                        LearnedSparseExpectedOutcome::Conflict
                    }
                };
                Ok(LearnedSparseBenchmarkCase {
                    case_id: case.case_id.clone(),
                    class: case.class,
                    query: case.query.clone(),
                    latency_budget_ms: case.budget.latency_ms,
                    memory_budget_bytes: case.budget.memory_bytes,
                    disk_budget_bytes: case.budget.disk_bytes,
                    ingest_update_budget_ms: case
                        .budget
                        .indexing_cost_micros
                        .max(case.budget.incremental_update_cost_micros)
                        .div_ceil(1_000),
                    energy_budget_millijoules: case.budget.energy_millijoules,
                    split: case.split,
                    fidelity: case.fidelity,
                    expected: Some(expected),
                })
            })
            .collect::<Result<Vec<_>, LearnedSparseCorpusError>>()?;
        let corpus = LearnedSparseBenchmarkCorpus {
            schema_version:
                crate::learned_sparse_benchmark::LEARNED_SPARSE_BENCHMARK_SCHEMA_VERSION,
            corpus_id: self.corpus_id.clone(),
            corpus_revision: self.corpus_revision.clone(),
            judgment_set_id: self.judgment_set_id.clone(),
            source_input_hash: self.source_input_hash.as_str().to_string(),
            evaluation_date: self.evaluation_date.clone(),
            cases,
            judgment_set_hash: Some(self.judgment_set_hash.clone()),
            environment,
            data_fidelity: LearnedSparseDataFidelity::RealMaestriaTask,
            corpus_snapshot: Some(corpus_snapshot),
            index_generation: Some(index_generation),
            namespace: Some(namespace),
            route_configurations,
        };
        corpus.validate().map_err(|error| {
            LearnedSparseCorpusError::InvalidCorpus(format!(
                "converted benchmark corpus is invalid: {error}"
            ))
        })?;
        Ok(corpus)
    }
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
