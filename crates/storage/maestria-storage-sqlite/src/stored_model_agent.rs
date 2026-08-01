//! Wire mirrors of `maestria_domain` model-agent records.
//!
//! This module is the façade for the model-agent wire mirrors: the
//! stage-result records (`StoredModelAgentSearchResult`,
//! `StoredModelAgentHarnessResult`, `StoredModelAgentValidationResult`,
//! `StoredModelAgentMemoryDecision`, `StoredModelAgentMemoryResult`) live
//! in `stored_model_agent_stages` and are re-exported here so consumers
//! keep importing from `crate::payloads::stored_model_agent`.

use maestria_domain::{
    ApprovalId, CorrelationId, EvidenceId, HarnessRunId, IndexGenerationId, JournalGeneration,
    ModelAgentProposalExecution, ModelAgentProposalRequest, ModelAgentProposalResult, TaskId,
};
use serde::{Deserialize, Serialize};

pub(crate) use super::stored_model_agent_stages::{
    StoredModelAgentHarnessResult, StoredModelAgentMemoryResult, StoredModelAgentSearchResult,
    StoredModelAgentValidationResult,
};

/// Wire mirror of `maestria_domain::ModelAgentProposalExecution`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredModelAgentProposalExecution {
    Fresh,
    JournalRecovery {
        journal_generation: u64,
    },
    ApprovalContinuation {
        approval_id: u64,
        journal_generation: u64,
    },
}

impl StoredModelAgentProposalExecution {
    pub(crate) fn from_domain(execution: &ModelAgentProposalExecution) -> Self {
        match execution {
            ModelAgentProposalExecution::Fresh => Self::Fresh,
            ModelAgentProposalExecution::JournalRecovery { journal_generation } => {
                Self::JournalRecovery {
                    journal_generation: journal_generation.value(),
                }
            }
            ModelAgentProposalExecution::ApprovalContinuation {
                approval_id,
                journal_generation,
            } => Self::ApprovalContinuation {
                approval_id: approval_id.value(),
                journal_generation: journal_generation.value(),
            },
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<ModelAgentProposalExecution, maestria_ports::PortError> {
        Ok(match self {
            Self::Fresh => ModelAgentProposalExecution::Fresh,
            Self::JournalRecovery { journal_generation } => {
                ModelAgentProposalExecution::JournalRecovery {
                    journal_generation: JournalGeneration::new(journal_generation),
                }
            }
            Self::ApprovalContinuation {
                approval_id,
                journal_generation,
            } => ModelAgentProposalExecution::ApprovalContinuation {
                approval_id: ApprovalId::new(approval_id),
                journal_generation: JournalGeneration::new(journal_generation),
            },
        })
    }
}

/// Wire mirror of `maestria_domain::ModelAgentProposalRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredModelAgentProposalRequest {
    pub(crate) run_id: u64,
    pub(crate) task_id: Option<u64>,
    pub(crate) query: String,
    pub(crate) limit: usize,
    pub(crate) evidence_ids: Vec<u64>,
    pub(crate) capability: String,
    pub(crate) command: String,
    pub(crate) working_directory: String,
    pub(crate) timeout_secs: u64,
    pub(crate) expected_generation: u64,
    pub(crate) task_validation: bool,
    pub(crate) memory_candidate: bool,
    pub(crate) execution: StoredModelAgentProposalExecution,
    pub(crate) correlation_id: u64,
}

impl StoredModelAgentProposalRequest {
    pub(crate) fn from_domain(request: &ModelAgentProposalRequest) -> Self {
        Self {
            run_id: request.run_id.value(),
            task_id: request.task_id.map(|id| id.value()),
            query: request.query.clone(),
            limit: request.limit,
            evidence_ids: request.evidence_ids.iter().map(|id| id.value()).collect(),
            capability: request.capability.clone(),
            command: request.command.clone(),
            working_directory: request.working_directory.clone(),
            timeout_secs: request.timeout_secs,
            expected_generation: request.expected_generation.value(),
            task_validation: request.task_validation,
            memory_candidate: request.memory_candidate,
            execution: StoredModelAgentProposalExecution::from_domain(&request.execution),
            correlation_id: request.correlation_id.value(),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<ModelAgentProposalRequest, maestria_ports::PortError> {
        Ok(ModelAgentProposalRequest {
            run_id: HarnessRunId::new(self.run_id),
            task_id: self.task_id.map(TaskId::new),
            query: self.query,
            limit: self.limit,
            evidence_ids: self.evidence_ids.into_iter().map(EvidenceId::new).collect(),
            capability: self.capability,
            command: self.command,
            working_directory: self.working_directory,
            timeout_secs: self.timeout_secs,
            expected_generation: IndexGenerationId::new(self.expected_generation),
            task_validation: self.task_validation,
            memory_candidate: self.memory_candidate,
            execution: self.execution.try_into_domain()?,
            correlation_id: CorrelationId::new(self.correlation_id),
        })
    }
}

/// Wire mirror of `maestria_domain::ModelAgentProposalResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredModelAgentProposalResult {
    Succeeded {
        run_id: u64,
        correlation_id: u64,
        search: Option<StoredModelAgentSearchResult>,
        harness: Option<StoredModelAgentHarnessResult>,
        validation: Option<StoredModelAgentValidationResult>,
        memory_candidate: Option<StoredModelAgentMemoryResult>,
    },
    Failed {
        run_id: u64,
        correlation_id: u64,
        error: String,
    },
}

impl StoredModelAgentProposalResult {
    pub(crate) fn from_domain(result: &ModelAgentProposalResult) -> Self {
        match result {
            ModelAgentProposalResult::Succeeded {
                run_id,
                correlation_id,
                search,
                harness,
                validation,
                memory_candidate,
            } => Self::Succeeded {
                correlation_id: correlation_id.value(),
                run_id: run_id.value(),
                search: search
                    .as_ref()
                    .map(StoredModelAgentSearchResult::from_domain),
                harness: harness
                    .as_ref()
                    .map(StoredModelAgentHarnessResult::from_domain),
                validation: validation
                    .as_ref()
                    .map(StoredModelAgentValidationResult::from_domain),
                memory_candidate: memory_candidate
                    .as_ref()
                    .map(StoredModelAgentMemoryResult::from_domain),
            },
            ModelAgentProposalResult::Failed {
                run_id,
                correlation_id,
                error,
            } => Self::Failed {
                run_id: run_id.value(),
                correlation_id: correlation_id.value(),
                error: error.clone(),
            },
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<ModelAgentProposalResult, maestria_ports::PortError> {
        Ok(match self {
            Self::Succeeded {
                run_id,
                correlation_id,
                search,
                harness,
                validation,
                memory_candidate,
            } => ModelAgentProposalResult::Succeeded {
                run_id: HarnessRunId::new(run_id),
                correlation_id: CorrelationId::new(correlation_id),
                search: search
                    .map(StoredModelAgentSearchResult::try_into_domain)
                    .transpose()?,
                harness: harness
                    .map(StoredModelAgentHarnessResult::try_into_domain)
                    .transpose()?,
                validation: validation
                    .map(StoredModelAgentValidationResult::try_into_domain)
                    .transpose()?,
                memory_candidate: memory_candidate
                    .map(StoredModelAgentMemoryResult::try_into_domain)
                    .transpose()?,
            },
            Self::Failed {
                run_id,
                correlation_id,
                error,
            } => ModelAgentProposalResult::Failed {
                run_id: HarnessRunId::new(run_id),
                correlation_id: CorrelationId::new(correlation_id),
                error,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{
        MemoryCandidateId, ModelAgentHarnessResult, ModelAgentMemoryDecision,
        ModelAgentMemoryResult, ModelAgentSearchResult, ModelAgentValidationResult, SearchTraceId,
    };

    fn request() -> ModelAgentProposalRequest {
        ModelAgentProposalRequest {
            run_id: HarnessRunId::new(1),
            task_id: Some(TaskId::new(2)),
            query: "what is a river?".to_string(),
            limit: 5,
            evidence_ids: vec![EvidenceId::new(3), EvidenceId::new(4)],
            capability: "bash".to_string(),
            command: "echo river".to_string(),
            working_directory: "/tmp".to_string(),
            timeout_secs: 30,
            expected_generation: IndexGenerationId::new(7),
            task_validation: true,
            memory_candidate: false,
            execution: ModelAgentProposalExecution::ApprovalContinuation {
                approval_id: ApprovalId::new(9),
                journal_generation: JournalGeneration::new(11),
            },
            correlation_id: CorrelationId::new(13),
        }
    }

    fn succeeded_result() -> ModelAgentProposalResult {
        ModelAgentProposalResult::Succeeded {
            run_id: HarnessRunId::new(1),
            correlation_id: CorrelationId::new(13),
            search: Some(ModelAgentSearchResult {
                trace_id: SearchTraceId::new(5),
                evidence_count: 3,
            }),
            harness: Some(ModelAgentHarnessResult {
                exit_code: 0,
                stdout: "ok".to_string(),
                stderr: String::new(),
                duration_ms: 120,
            }),
            validation: Some(ModelAgentValidationResult {
                passed: true,
                warnings: vec!["minor".to_string()],
            }),
            memory_candidate: Some(ModelAgentMemoryResult {
                candidate_id: MemoryCandidateId::new(8),
                confidence_milli: 900,
                decision: ModelAgentMemoryDecision::Promote,
            }),
        }
    }

    #[test]
    fn request_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = request();
        let stored = StoredModelAgentProposalRequest::from_domain(&original);
        let json = serde_json::to_string(&stored)?;
        let decoded = serde_json::from_str::<StoredModelAgentProposalRequest>(&json)?;
        let restored = decoded.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn succeeded_result_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = succeeded_result();
        let stored = StoredModelAgentProposalResult::from_domain(&original);
        let json = serde_json::to_string(&stored)?;
        let decoded = serde_json::from_str::<StoredModelAgentProposalResult>(&json)?;
        let restored = decoded.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn failed_result_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = ModelAgentProposalResult::Failed {
            run_id: HarnessRunId::new(2),
            correlation_id: CorrelationId::new(14),
            error: "harness crashed".to_string(),
        };
        let stored = StoredModelAgentProposalResult::from_domain(&original);
        let restored = stored.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn unknown_request_field_is_rejected_during_deserialization()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut value =
            serde_json::to_value(StoredModelAgentProposalRequest::from_domain(&request()))?;
        value
            .as_object_mut()
            .ok_or_else(|| "expected JSON object".to_string())?
            .insert("extra".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<StoredModelAgentProposalRequest>(value).is_err());
        Ok(())
    }

    #[test]
    fn result_serializes_with_snake_case_variant_names() -> Result<(), Box<dyn std::error::Error>> {
        let stored = StoredModelAgentProposalResult::from_domain(&succeeded_result());
        let json = serde_json::to_string(&stored)?;
        assert!(json.contains("\"succeeded\""));
        assert!(json.contains("\"memory_candidate\""));
        Ok(())
    }
}
