//! Wire mirrors of the model-agent stage-result records of
//! `maestria_domain`: `ModelAgentSearchResult`, `ModelAgentHarnessResult`,
//! `ModelAgentValidationResult`, `ModelAgentMemoryDecision` and
//! `ModelAgentMemoryResult`.
//!
//! These types serialize the per-stage outcomes of a model-agent proposal
//! run; the proposal request/result records themselves live in
//! `stored_model_agent` and embed these stage results.

use maestria_domain::{
    MemoryCandidateId, ModelAgentHarnessResult, ModelAgentMemoryDecision, ModelAgentMemoryResult,
    ModelAgentSearchResult, ModelAgentValidationResult, SearchTraceId,
};
use serde::{Deserialize, Serialize};

/// Wire mirror of `maestria_domain::ModelAgentSearchResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredModelAgentSearchResult {
    pub(crate) trace_id: u64,
    pub(crate) evidence_count: usize,
}

impl StoredModelAgentSearchResult {
    pub(crate) fn from_domain(result: &ModelAgentSearchResult) -> Self {
        Self {
            trace_id: result.trace_id.value(),
            evidence_count: result.evidence_count,
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<ModelAgentSearchResult, maestria_ports::PortError> {
        Ok(ModelAgentSearchResult {
            trace_id: SearchTraceId::new(self.trace_id),
            evidence_count: self.evidence_count,
        })
    }
}

/// Wire mirror of `maestria_domain::ModelAgentHarnessResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredModelAgentHarnessResult {
    pub(crate) exit_code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) duration_ms: u64,
}

impl StoredModelAgentHarnessResult {
    pub(crate) fn from_domain(result: &ModelAgentHarnessResult) -> Self {
        Self {
            exit_code: result.exit_code,
            stdout: result.stdout.clone(),
            stderr: result.stderr.clone(),
            duration_ms: result.duration_ms,
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<ModelAgentHarnessResult, maestria_ports::PortError> {
        Ok(ModelAgentHarnessResult {
            exit_code: self.exit_code,
            stdout: self.stdout,
            stderr: self.stderr,
            duration_ms: self.duration_ms,
        })
    }
}

/// Wire mirror of `maestria_domain::ModelAgentValidationResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredModelAgentValidationResult {
    pub(crate) passed: bool,
    pub(crate) warnings: Vec<String>,
}

impl StoredModelAgentValidationResult {
    pub(crate) fn from_domain(result: &ModelAgentValidationResult) -> Self {
        Self {
            passed: result.passed,
            warnings: result.warnings.clone(),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<ModelAgentValidationResult, maestria_ports::PortError> {
        Ok(ModelAgentValidationResult {
            passed: self.passed,
            warnings: self.warnings,
        })
    }
}

/// Wire mirror of `maestria_domain::ModelAgentMemoryDecision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredModelAgentMemoryDecision {
    Promote,
    RequireEvidence,
    RequireReview,
    Deny,
}

impl StoredModelAgentMemoryDecision {
    pub(crate) fn from_domain(decision: ModelAgentMemoryDecision) -> Self {
        match decision {
            ModelAgentMemoryDecision::Promote => Self::Promote,
            ModelAgentMemoryDecision::RequireEvidence => Self::RequireEvidence,
            ModelAgentMemoryDecision::RequireReview => Self::RequireReview,
            ModelAgentMemoryDecision::Deny => Self::Deny,
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<ModelAgentMemoryDecision, maestria_ports::PortError> {
        Ok(match self {
            Self::Promote => ModelAgentMemoryDecision::Promote,
            Self::RequireEvidence => ModelAgentMemoryDecision::RequireEvidence,
            Self::RequireReview => ModelAgentMemoryDecision::RequireReview,
            Self::Deny => ModelAgentMemoryDecision::Deny,
        })
    }
}

/// Wire mirror of `maestria_domain::ModelAgentMemoryResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredModelAgentMemoryResult {
    pub(crate) candidate_id: u64,
    pub(crate) confidence_milli: u16,
    pub(crate) decision: StoredModelAgentMemoryDecision,
}

impl StoredModelAgentMemoryResult {
    pub(crate) fn from_domain(result: &ModelAgentMemoryResult) -> Self {
        Self {
            candidate_id: result.candidate_id.value(),
            confidence_milli: result.confidence_milli,
            decision: StoredModelAgentMemoryDecision::from_domain(result.decision),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<ModelAgentMemoryResult, maestria_ports::PortError> {
        Ok(ModelAgentMemoryResult {
            candidate_id: MemoryCandidateId::new(self.candidate_id),
            confidence_milli: self.confidence_milli,
            decision: self.decision.try_into_domain()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_memory_decision_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        for decision in [
            ModelAgentMemoryDecision::Promote,
            ModelAgentMemoryDecision::RequireEvidence,
            ModelAgentMemoryDecision::RequireReview,
            ModelAgentMemoryDecision::Deny,
        ] {
            let stored = StoredModelAgentMemoryDecision::from_domain(decision);
            assert_eq!(stored.try_into_domain()?, decision);
        }
        Ok(())
    }
}
