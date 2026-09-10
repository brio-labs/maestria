use std::collections::{BTreeMap, BTreeSet};

use maestria_domain::{ContentHash, SearchIntent};
use serde::{Deserialize, Serialize};

use super::{
    LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION, LateInteractionBenchmarkCorpus,
    LateInteractionBenchmarkError, LateInteractionClassDecision, LateInteractionObservation,
    LateInteractionStage, LateInteractionStageAComparison,
};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionStageAReportCorpus {
    pub id: String,
    pub revision: String,
    pub judgment_set: String,
    pub source_hash: ContentHash,
    pub judgment_hash: ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionStageAFingerprints {
    pub corpus_snapshot: String,
    pub index_generation: String,
    pub identity_digest: String,
    pub profile_digest: String,
    pub scorer_fingerprint: String,
    pub source_hash: String,
    pub judgment_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionStageAReport {
    pub schema_version: u32,
    pub measurement_kind: String,
    pub evaluation_date: String,
    pub evaluation_id: String,
    pub stage: LateInteractionStage,
    pub data_fidelity: String,
    pub corpus: LateInteractionStageAReportCorpus,
    pub fingerprints: LateInteractionStageAFingerprints,
    pub observations: Vec<LateInteractionObservation>,
    pub decisions: BTreeMap<String, String>,
    pub promotion: LateInteractionStageAReportPromotion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionStageAReportPromotion {
    pub authorized: bool,
    pub reason: String,
}

impl LateInteractionStageAReport {
    pub fn from_observations(
        corpus: &LateInteractionBenchmarkCorpus,
        evaluation_date: impl Into<String>,
        evaluation_id: impl Into<String>,
        data_fidelity: impl Into<String>,
        fingerprints: LateInteractionStageAFingerprints,
        observations: Vec<LateInteractionObservation>,
    ) -> Result<Self, LateInteractionBenchmarkError> {
        let comparison = LateInteractionStageAComparison::evaluate(corpus, &observations)?;
        let data_fidelity = data_fidelity.into();
        if !matches!(data_fidelity.as_str(), "real" | "mixed" | "staged") {
            return Err(LateInteractionBenchmarkError::InvalidCorpus(
                "Stage A data fidelity must be real, mixed, or staged",
            ));
        }
        let decisions = comparison
            .classes
            .iter()
            .map(|(class, comparison)| {
                let name = match comparison.decision {
                    LateInteractionClassDecision::RetainBaseline { .. } => "RetainBaseline",
                    LateInteractionClassDecision::RetainLateInteractionReranker { .. } => {
                        "RetainLateInteractionReranker"
                    }
                    LateInteractionClassDecision::RejectExperiment { .. } => "RejectExperiment",
                };
                (format!("{class:?}"), name.to_owned())
            })
            .collect();
        let authorized = data_fidelity == "real" && !comparison.winning_classes().is_empty();
        let report = Self {
            schema_version: LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION,
            measurement_kind: "late-interaction-stage-a".to_owned(),
            evaluation_date: evaluation_date.into(),
            evaluation_id: evaluation_id.into(),
            stage: LateInteractionStage::Reranker,
            data_fidelity,
            corpus: LateInteractionStageAReportCorpus {
                id: corpus.corpus_id.clone(),
                revision: corpus.revision.clone(),
                judgment_set: corpus.judgment_set.clone(),
                source_hash: corpus.source_hash.clone(),
                judgment_hash: corpus.judgment_hash.clone(),
            },
            fingerprints,
            observations,
            decisions,
            promotion: LateInteractionStageAReportPromotion {
                authorized,
                reason: if authorized {
                    "measured material quality win without regression".to_owned()
                } else {
                    "no measured material quality win without regression".to_owned()
                },
            },
        };
        report.validate_against_corpus(corpus)?;
        Ok(report)
    }

    pub fn validate_against_corpus(
        &self,
        corpus: &LateInteractionBenchmarkCorpus,
    ) -> Result<LateInteractionStageAComparison, LateInteractionBenchmarkError> {
        if self.schema_version != LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION
            || self.measurement_kind != "late-interaction-stage-a"
            || self.evaluation_date.trim().is_empty()
            || self.evaluation_id.trim().is_empty()
            || self.stage != LateInteractionStage::Reranker
            || !matches!(self.data_fidelity.as_str(), "real" | "mixed" | "staged")
        {
            return Err(LateInteractionBenchmarkError::InvalidReport(
                "Stage A report identity or stage is invalid",
            ));
        }
        if self.corpus.id != corpus.corpus_id
            || self.corpus.revision != corpus.revision
            || self.corpus.judgment_set != corpus.judgment_set
            || self.corpus.source_hash != corpus.source_hash
            || self.corpus.judgment_hash != corpus.judgment_hash
            || self.fingerprints.source_hash != corpus.source_hash.as_str()
            || self.fingerprints.judgment_hash != corpus.judgment_hash.as_str()
        {
            return Err(LateInteractionBenchmarkError::ReportIdentityMismatch);
        }
        let comparison = LateInteractionStageAComparison::evaluate(corpus, &self.observations)?;
        let expected_decisions = comparison
            .classes
            .iter()
            .map(|(class, comparison)| {
                let decision = match comparison.decision {
                    LateInteractionClassDecision::RetainBaseline { .. } => "RetainBaseline",
                    LateInteractionClassDecision::RetainLateInteractionReranker { .. } => {
                        "RetainLateInteractionReranker"
                    }
                    LateInteractionClassDecision::RejectExperiment { .. } => "RejectExperiment",
                };
                (format!("{class:?}"), decision.to_owned())
            })
            .collect::<BTreeMap<_, _>>();
        if self.decisions != expected_decisions {
            return Err(LateInteractionBenchmarkError::InvalidReport(
                "Stage A decisions do not match measured observations",
            ));
        }
        let expected_authorized =
            self.data_fidelity == "real" && !comparison.winning_classes().is_empty();
        if self.promotion.authorized != expected_authorized {
            return Err(LateInteractionBenchmarkError::InvalidReport(
                "Stage A promotion authorization does not match measured observations",
            ));
        }
        Ok(comparison)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionStageAPromotionRecord {
    pub schema_version: u32,
    pub evaluation_id: String,
    pub evaluation_date: String,
    pub corpus_id: String,
    pub corpus_revision: String,
    pub stage_a_report_hash: ContentHash,
    pub profile_identity: String,
    pub generation_id: String,
    pub corpus_snapshot: String,
    pub rollback_generation_id: String,
    pub promoted_classes: BTreeSet<SearchIntent>,
    pub authorized: bool,
}

impl LateInteractionStageAPromotionRecord {
    pub fn from_stage_a(
        stage_a: &LateInteractionStageAReport,
        corpus: &LateInteractionBenchmarkCorpus,
        profile_identity: impl Into<String>,
        generation_id: impl Into<String>,
        corpus_snapshot: impl Into<String>,
        rollback_generation_id: impl Into<String>,
    ) -> Result<Self, LateInteractionBenchmarkError> {
        let comparison = stage_a.validate_against_corpus(corpus)?;
        let profile_identity = profile_identity.into();
        let generation_id = generation_id.into();
        let corpus_snapshot = corpus_snapshot.into();
        let rollback_generation_id = rollback_generation_id.into();
        let profile_identity_valid = ContentHash::new(profile_identity.clone()).is_ok();
        if stage_a.data_fidelity != "real"
            || !stage_a.promotion.authorized
            || comparison.winning_classes().is_empty()
            || !profile_identity_valid
            || generation_id.trim().is_empty()
            || corpus_snapshot.trim().is_empty()
            || rollback_generation_id.trim().is_empty()
        {
            return Err(LateInteractionBenchmarkError::InvalidReport(
                "Stage A evidence is not eligible for opt-in promotion",
            ));
        }
        let record = Self {
            schema_version: LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION,
            evaluation_id: stage_a.evaluation_id.clone(),
            evaluation_date: stage_a.evaluation_date.clone(),
            corpus_id: corpus.corpus_id.clone(),
            corpus_revision: corpus.revision.clone(),
            stage_a_report_hash: hash_stage_a_report(stage_a)?,
            profile_identity,
            generation_id,
            corpus_snapshot,
            rollback_generation_id,
            promoted_classes: comparison.winning_classes(),
            authorized: true,
        };
        record.validate_against_stage_a(stage_a, corpus)?;
        Ok(record)
    }
    pub fn validate_against_stage_a(
        &self,
        stage_a: &LateInteractionStageAReport,
        corpus: &LateInteractionBenchmarkCorpus,
    ) -> Result<(), LateInteractionBenchmarkError> {
        self.validate_against_report(stage_a)?;
        let comparison = stage_a.validate_against_corpus(corpus)?;
        if self.promoted_classes != comparison.winning_classes() {
            return Err(LateInteractionBenchmarkError::InvalidReport(
                "Stage A promotion classes do not match measured decisions",
            ));
        }
        Ok(())
    }

    pub fn validate_against_report(
        &self,
        stage_a: &LateInteractionStageAReport,
    ) -> Result<(), LateInteractionBenchmarkError> {
        let expected_classes = [
            SearchIntent::FactualLocal,
            SearchIntent::SemanticDiscovery,
            SearchIntent::CompositionalConstraints,
            SearchIntent::RepositoryCode,
        ]
        .into_iter()
        .filter(|class| {
            stage_a
                .decisions
                .get(&format!("{class:?}"))
                .is_some_and(|decision| decision == "RetainLateInteractionReranker")
        })
        .collect::<BTreeSet<_>>();
        if self.schema_version != LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION
            || self.evaluation_id != stage_a.evaluation_id
            || self.evaluation_date != stage_a.evaluation_date
            || self.corpus_id != stage_a.corpus.id
            || self.corpus_revision != stage_a.corpus.revision
            || self.stage_a_report_hash != hash_stage_a_report(stage_a)?
            || self.profile_identity.trim().is_empty()
            || ContentHash::new(self.profile_identity.clone()).is_err()
            || self.generation_id.trim().is_empty()
            || self.corpus_snapshot.trim().is_empty()
            || self.rollback_generation_id.trim().is_empty()
            || self.promoted_classes != expected_classes
            || self.promoted_classes.is_empty()
            || !self.authorized
            || stage_a.data_fidelity != "real"
            || !stage_a.promotion.authorized
        {
            return Err(LateInteractionBenchmarkError::InvalidReport(
                "Stage A promotion record identity or authorization is invalid",
            ));
        }
        Ok(())
    }
}

pub(super) fn hash_stage_a_report(
    report: &LateInteractionStageAReport,
) -> Result<ContentHash, LateInteractionBenchmarkError> {
    let bytes = serde_json::to_vec(report).map_err(|_| {
        LateInteractionBenchmarkError::InvalidReport("Stage A report serialization failed")
    })?;
    ContentHash::new(maestria_domain::content_hash(&bytes))
        .map_err(|_| LateInteractionBenchmarkError::InvalidReport("Stage A report hash failed"))
}
