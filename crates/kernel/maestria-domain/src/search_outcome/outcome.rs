use serde::{Deserialize, Serialize};

use crate::ids::{ConflictSetId, IndexGenerationId, SearchTraceId};
use crate::search::RetrievalModelFingerprint;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictSet {
    pub id: ConflictSetId,
    pub candidates: Vec<super::EvidenceCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchStatus {
    Answerable,
    AnswerableWithWarnings,
    EvidenceIncomplete,
    SourcesConflict,
    StaleEvidenceOnly,
    NoEvidenceFound,
    Abstained,
    DeniedByPolicy,
    QuarantinedForReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOutcome {
    pub trace: SearchTraceId,
    #[serde(default)]
    pub trace_data: Option<Box<super::SearchTrace>>,
    pub fingerprint: RetrievalModelFingerprint,
    pub index_generation: IndexGenerationId,
    pub status: SearchStatus,
    pub evidence: Vec<super::EvidenceCandidate>,
    pub coverage: super::EvidenceCoverage,
    pub conflicts: Vec<ConflictSet>,
}

impl SearchOutcome {
    pub fn canonicalize_score_provenance(
        &mut self,
    ) -> Result<(), crate::search::SearchCompatibilityError> {
        for candidate in &mut self.evidence {
            candidate.canonicalize_score_provenance()?;
        }
        for conflict in &mut self.conflicts {
            for candidate in &mut conflict.candidates {
                candidate.canonicalize_score_provenance()?;
            }
        }
        if let Some(trace) = &mut self.trace_data {
            trace.canonicalize_score_provenance()?;
            self.trace = trace.deterministic_id();
        }
        Ok(())
    }
}
