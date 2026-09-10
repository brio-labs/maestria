use std::collections::BTreeSet;

use maestria_domain::{ContentHash, SearchIntent};
use serde::{Deserialize, Serialize};

use super::{LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION, LateInteractionBenchmarkError};
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LateInteractionStage {
    Reranker,
    CandidateIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LateInteractionRoute {
    LexicalExact,
    EligibleHybrid,
    EligibleBoundedBaseline,
    LateInteractionReranker,
    MultiVectorCandidate,
}

impl LateInteractionRoute {
    pub const fn all_stage_a() -> [Self; 4] {
        [
            Self::LexicalExact,
            Self::EligibleHybrid,
            Self::EligibleBoundedBaseline,
            Self::LateInteractionReranker,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionBenchmarkJudgment {
    pub source: String,
    pub grade: u32,
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionBenchmarkCase {
    pub case_id: String,
    pub query: String,
    pub query_class: SearchIntent,
    pub tags: BTreeSet<String>,
    pub relevant_evidence_ids: BTreeSet<u64>,
    pub source_file: String,
    pub judgments: Vec<LateInteractionBenchmarkJudgment>,
    pub latency_budget_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionBenchmarkCorpus {
    pub schema_version: u32,
    pub corpus_id: String,
    pub revision: String,
    pub judgment_set: String,
    pub source_hash: ContentHash,
    pub judgment_hash: ContentHash,
    pub source_paths: Vec<String>,
    pub cases: Vec<LateInteractionBenchmarkCase>,
}

impl LateInteractionBenchmarkCorpus {
    pub fn validate(&self) -> Result<(), LateInteractionBenchmarkError> {
        if self.schema_version != LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION
            || self.corpus_id.trim().is_empty()
            || self.revision.trim().is_empty()
            || self.judgment_set.trim().is_empty()
            || self.source_paths.is_empty()
            || self.source_paths.iter().any(|path| path.trim().is_empty())
            || self.cases.is_empty()
        {
            return Err(LateInteractionBenchmarkError::InvalidCorpus(
                "corpus identity, source paths, and cases must be non-empty",
            ));
        }
        let mut ids = BTreeSet::new();
        for case in &self.cases {
            if case.case_id.trim().is_empty()
                || case.query.trim().is_empty()
                || case.source_file.trim().is_empty()
                || case.latency_budget_ms == 0
                || !ids.insert(case.case_id.clone())
            {
                return Err(LateInteractionBenchmarkError::InvalidCorpus(
                    "case identity, query, source file, latency budget, and uniqueness are required",
                ));
            }
            if !self
                .source_paths
                .iter()
                .any(|path| path.ends_with(&case.source_file))
            {
                return Err(LateInteractionBenchmarkError::InvalidCorpus(
                    "case source file is not listed in corpus source paths",
                ));
            }
            for judgment in &case.judgments {
                if judgment.source != case.source_file
                    || judgment.start > judgment.end
                    || judgment.source.trim().is_empty()
                {
                    return Err(LateInteractionBenchmarkError::InvalidCorpus(
                        "judgment source and offsets are invalid",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn from_json(input: &str) -> Result<Self, LateInteractionBenchmarkError> {
        let corpus: Self = serde_json::from_str(input)
            .map_err(|error| LateInteractionBenchmarkError::InvalidJson(error.to_string()))?;
        corpus.validate()?;
        Ok(corpus)
    }

    pub fn case(&self, case_id: &str) -> Option<&LateInteractionBenchmarkCase> {
        self.cases.iter().find(|case| case.case_id == case_id)
    }
}
