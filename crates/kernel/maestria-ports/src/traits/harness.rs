use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessCommandClass {
    Shell,
    Browser,
    Fetch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessCapabilities {
    pub command_classes: Vec<HarnessCommandClass>,
    pub write_enabled: bool,
    pub read_enabled: bool,
    pub web_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessRequest {
    pub run_id: maestria_domain::HarnessRunId,
    pub command: String,
    pub working_directory: PathBuf,
    pub duration_budget: Duration,
    pub class: HarnessCommandClass,
    pub readable_roots: Vec<PathBuf>,
    pub blocked_paths: Vec<PathBuf>,
    pub blocked_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessOutcome {
    pub run_id: maestria_domain::HarnessRunId,
    pub command: String,
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration: Duration,
    pub artifacts_created: Vec<maestria_domain::BlobId>,
    pub diff_summary: Option<String>,
    pub validation_hints: Vec<String>,
}

pub trait HarnessAdapter: Send + Sync {
    fn capabilities(&self) -> Result<HarnessCapabilities, crate::PortError>;
    fn execute(
        &self,
        request: HarnessRequest,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<HarnessOutcome, crate::PortError>> + Send + '_>>;
}

/// Untrusted proposal submitted by a model-facing adapter.
///
/// Validation produces a governed [`HarnessRequest`]; the proposal itself
/// never executes commands or mutates domain state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAgentProposal {
    pub run_id: maestria_domain::HarnessRunId,
    pub task_id: Option<maestria_domain::TaskId>,
    pub query: String,
    pub limit: usize,
    pub capability: String,
    pub command: String,
    pub working_directory: PathBuf,
    pub timeout: Duration,
    pub expected_generation: u64,
    pub evidence_ids: Vec<maestria_domain::EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernedAgentProposal {
    pub search_query: String,
    pub search_limit: usize,
    pub evidence_ids: Vec<maestria_domain::EvidenceId>,
    pub harness: HarnessRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelAgentProposalError {
    EmptyQuery,
    QueryTooLong,
    InvalidLimit,
    EmptyCommand,
    CommandTooLong,
    InvalidTimeout,
    UnsupportedCapability,
    StaleGeneration { expected: u64, current: u64 },
    TooManyEvidenceIds,
    UnknownEvidence(maestria_domain::EvidenceId),
}

impl std::fmt::Display for ModelAgentProposalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyQuery => write!(formatter, "model query is empty"),
            Self::QueryTooLong => write!(formatter, "model query exceeds 4096 bytes"),
            Self::InvalidLimit => write!(formatter, "model search limit must be 1..=100"),
            Self::EmptyCommand => write!(formatter, "model harness command is empty"),
            Self::CommandTooLong => write!(formatter, "model harness command exceeds 4096 bytes"),
            Self::InvalidTimeout => write!(formatter, "model timeout must be 1..=120 seconds"),
            Self::UnsupportedCapability => write!(
                formatter,
                "model requested an unsupported harness capability"
            ),
            Self::StaleGeneration { expected, current } => {
                write!(
                    formatter,
                    "model proposal is stale: expected generation {expected}, current {current}"
                )
            }
            Self::TooManyEvidenceIds => {
                write!(formatter, "model supplied more than 100 evidence ids")
            }
            Self::UnknownEvidence(id) => {
                write!(formatter, "model referenced unknown evidence {id}")
            }
        }
    }
}

impl std::error::Error for ModelAgentProposalError {}

impl ModelAgentProposal {
    pub fn validate(
        &self,
        current_generation: u64,
        available_evidence: &std::collections::BTreeSet<maestria_domain::EvidenceId>,
    ) -> Result<GovernedAgentProposal, ModelAgentProposalError> {
        if self.query.trim().is_empty() {
            return Err(ModelAgentProposalError::EmptyQuery);
        }
        if self.query.len() > 4096 {
            return Err(ModelAgentProposalError::QueryTooLong);
        }
        if !(1..=100).contains(&self.limit) {
            return Err(ModelAgentProposalError::InvalidLimit);
        }
        if self.command.trim().is_empty() {
            return Err(ModelAgentProposalError::EmptyCommand);
        }
        if self.command.len() > 4096 {
            return Err(ModelAgentProposalError::CommandTooLong);
        }
        if !(1..=120).contains(&self.timeout.as_secs()) {
            return Err(ModelAgentProposalError::InvalidTimeout);
        }
        if !matches!(
            self.capability.as_str(),
            "shell" | "browser" | "fetch" | "web"
        ) {
            return Err(ModelAgentProposalError::UnsupportedCapability);
        }
        if self.expected_generation != current_generation {
            return Err(ModelAgentProposalError::StaleGeneration {
                expected: self.expected_generation,
                current: current_generation,
            });
        }
        if self.evidence_ids.len() > 100 {
            return Err(ModelAgentProposalError::TooManyEvidenceIds);
        }
        if let Some(id) = self
            .evidence_ids
            .iter()
            .find(|id| !available_evidence.contains(id))
        {
            return Err(ModelAgentProposalError::UnknownEvidence(*id));
        }
        let class = match self.capability.as_str() {
            "shell" => HarnessCommandClass::Shell,
            "browser" => HarnessCommandClass::Browser,
            "fetch" | "web" => HarnessCommandClass::Fetch,
            _ => return Err(ModelAgentProposalError::UnsupportedCapability),
        };
        Ok(GovernedAgentProposal {
            search_query: self.query.clone(),
            search_limit: self.limit,
            evidence_ids: self.evidence_ids.clone(),
            harness: HarnessRequest {
                run_id: self.run_id,
                command: self.command.clone(),
                working_directory: self.working_directory.clone(),
                duration_budget: self.timeout,
                class,
                readable_roots: Vec::new(),
                blocked_paths: Vec::new(),
                blocked_patterns: Vec::new(),
            },
        })
    }
}

#[cfg(test)]
mod model_boundary_tests {
    use super::*;
    use std::collections::BTreeSet;

    fn proposal() -> ModelAgentProposal {
        ModelAgentProposal {
            run_id: maestria_domain::HarnessRunId::new(1),
            task_id: Some(maestria_domain::TaskId::new(2)),
            query: "find the validation gate".into(),
            limit: 10,
            capability: "shell".into(),
            command: "cargo test -p maestria-domain".into(),
            working_directory: PathBuf::from("."),
            timeout: Duration::from_secs(30),
            expected_generation: 4,
            evidence_ids: vec![maestria_domain::EvidenceId::new(9)],
        }
    }

    #[test]
    fn valid_proposal_becomes_governed_harness_request() -> Result<(), Box<dyn std::error::Error>> {
        let available = BTreeSet::from([maestria_domain::EvidenceId::new(9)]);
        let governed = proposal().validate(4, &available)?;
        assert_eq!(governed.harness.class, HarnessCommandClass::Shell);
        assert_eq!(governed.search_limit, 10);
        Ok(())
    }

    #[test]
    fn stale_and_unbounded_proposals_are_rejected() {
        let available = BTreeSet::from([maestria_domain::EvidenceId::new(9)]);
        assert!(matches!(
            proposal().validate(5, &available),
            Err(ModelAgentProposalError::StaleGeneration { .. })
        ));
        let mut too_large = proposal();
        too_large.query = "x".repeat(4097);
        assert_eq!(
            too_large.validate(4, &available),
            Err(ModelAgentProposalError::QueryTooLong)
        );
    }
}
