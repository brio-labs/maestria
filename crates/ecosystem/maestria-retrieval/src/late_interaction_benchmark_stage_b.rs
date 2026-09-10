use std::collections::BTreeSet;

use maestria_domain::ContentHash;
use serde::{Deserialize, Serialize};

use super::{
    LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION, LateInteractionBenchmarkCorpus,
    LateInteractionBenchmarkError, LateInteractionStage, LateInteractionStageAComparison,
    LateInteractionStageAReport, reports::hash_stage_a_report,
};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexedRetrievalNeed {
    MeasuredNeed {
        case_ids: Vec<String>,
        missing_relevant_count: u32,
        source_encoding_bottleneck_cases: Vec<String>,
    },
    NoMeasuredNeed {
        reason: String,
    },
    Unavailable {
        reason: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionStageBReport {
    pub schema_version: u32,
    pub measurement_kind: String,
    pub evaluation_date: String,
    pub evaluation_id: String,
    pub stage: LateInteractionStage,
    pub stage_a_report_hash: ContentHash,
    pub stage_a_quality_win: bool,
    pub indexed_need: IndexedRetrievalNeed,
    pub decision: LateInteractionStageBDecision,
    pub promotion: LateInteractionStageBPromotion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionStageBPromotion {
    pub authorized: bool,
    pub index_implemented: bool,
}

impl LateInteractionStageBReport {
    pub fn from_stage_a(
        stage_a: &LateInteractionStageAReport,
        corpus: &LateInteractionBenchmarkCorpus,
        evaluation_date: impl Into<String>,
        evaluation_id: impl Into<String>,
        indexed_need: IndexedRetrievalNeed,
    ) -> Result<Self, LateInteractionBenchmarkError> {
        let comparison = stage_a.validate_against_corpus(corpus)?;
        let stage_a_quality_win = stage_a.data_fidelity == "real"
            && stage_a.promotion.authorized
            && !comparison.winning_classes().is_empty();
        let decision = decide_stage_b(&comparison, &indexed_need);
        let report = Self {
            schema_version: LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION,
            measurement_kind: "late-interaction-stage-b".to_owned(),
            evaluation_date: evaluation_date.into(),
            evaluation_id: evaluation_id.into(),
            stage: LateInteractionStage::CandidateIndex,
            stage_a_report_hash: hash_stage_a_report(stage_a)?,
            stage_a_quality_win,
            indexed_need,
            decision,
            promotion: LateInteractionStageBPromotion {
                authorized: false,
                index_implemented: false,
            },
        };
        report.validate_against_stage_a(stage_a, corpus)?;
        Ok(report)
    }

    pub fn validate_against_stage_a(
        &self,
        stage_a: &LateInteractionStageAReport,
        corpus: &LateInteractionBenchmarkCorpus,
    ) -> Result<(), LateInteractionBenchmarkError> {
        let comparison = stage_a.validate_against_corpus(corpus)?;
        self.indexed_need.validate_against_corpus(corpus)?;
        if self.schema_version != LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION
            || self.measurement_kind != "late-interaction-stage-b"
            || self.evaluation_date.trim().is_empty()
            || self.evaluation_id.trim().is_empty()
            || self.stage != LateInteractionStage::CandidateIndex
            || self.stage_a_report_hash != hash_stage_a_report(stage_a)?
        {
            return Err(LateInteractionBenchmarkError::InvalidReport(
                "Stage B report identity or stage is invalid",
            ));
        }
        let expected_quality_win = stage_a.data_fidelity == "real"
            && stage_a.promotion.authorized
            && !comparison.winning_classes().is_empty();
        if self.stage_a_quality_win != expected_quality_win
            || self.decision != decide_stage_b(&comparison, &self.indexed_need)
            || self.promotion.authorized
            || self.promotion.index_implemented
        {
            return Err(LateInteractionBenchmarkError::InvalidReport(
                "Stage B authorization does not match Stage A and indexed-need gates",
            ));
        }
        Ok(())
    }
}

impl IndexedRetrievalNeed {
    fn validate_against_corpus(
        &self,
        corpus: &LateInteractionBenchmarkCorpus,
    ) -> Result<(), LateInteractionBenchmarkError> {
        let valid_case_ids = |case_ids: &[String]| {
            let mut unique = BTreeSet::new();
            !case_ids.is_empty()
                && case_ids.iter().all(|case_id| {
                    !case_id.trim().is_empty()
                        && corpus.case(case_id).is_some()
                        && unique.insert(case_id)
                })
        };
        match self {
            Self::MeasuredNeed {
                case_ids,
                source_encoding_bottleneck_cases,
                ..
            } if valid_case_ids(case_ids)
                && (source_encoding_bottleneck_cases.is_empty()
                    || valid_case_ids(source_encoding_bottleneck_cases)) =>
            {
                Ok(())
            }
            Self::NoMeasuredNeed { reason } | Self::Unavailable { reason }
                if !reason.trim().is_empty() =>
            {
                Ok(())
            }
            _ => Err(LateInteractionBenchmarkError::InvalidReport(
                "indexed-retrieval need must reference measured corpus cases",
            )),
        }
    }

    fn authorizes(&self) -> bool {
        matches!(
            self,
            Self::MeasuredNeed {
                case_ids,
                missing_relevant_count,
                source_encoding_bottleneck_cases,
            } if case_ids.len() >= 2
                && (*missing_relevant_count > 0 || source_encoding_bottleneck_cases.len() >= 2)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LateInteractionStageBDecision {
    NotAuthorized { reason: String },
    AuthorizedForEvaluation { reason: String },
    RetainStageA { reason: String },
    PromoteMultiVectorIndex { reason: String },
    RejectIndex { reason: String },
}

pub fn decide_stage_b(
    comparison: &LateInteractionStageAComparison,
    need: &IndexedRetrievalNeed,
) -> LateInteractionStageBDecision {
    if comparison.winning_classes().is_empty() {
        return LateInteractionStageBDecision::NotAuthorized {
            reason: "Stage A has no measured winning class".into(),
        };
    }
    if !need.authorizes() {
        return LateInteractionStageBDecision::NotAuthorized {
            reason: "bounded Stage A has no measured indexed-retrieval need".into(),
        };
    }
    LateInteractionStageBDecision::AuthorizedForEvaluation {
        reason: "Stage A win and measured indexed-retrieval need are both present".into(),
    }
}
