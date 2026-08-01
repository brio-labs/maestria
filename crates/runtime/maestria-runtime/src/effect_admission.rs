use crate::config::EffectExecutionContext;
use crate::proposal_persistence::decode_pending_continuation;
use crate::proposal_recovery::journal_entry_matches_proposal;
use maestria_domain::{MaestriaEffect, ModelAgentProposalExecution};
use maestria_governance::{ApprovalRequest, PolicyDecision, RiskClass, ScopeGuard};
use maestria_ports::{ApprovalStatus, EffectJournalEntry, EffectJournalStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ApprovedProposalClaim {
    pub(crate) run_id: maestria_domain::HarnessRunId,
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalWait {
    CreateProposalApproval,
    ExistingProposalApproval,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RejectionCause {
    Reason(String),
    ApprovalLookup(maestria_ports::PortError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RejectionHandling {
    ObserveOnly,
    LegacyHarness(Box<maestria_domain::QueryHarnessRequest>),
    Proposal(Box<maestria_domain::ModelAgentProposalRequest>),
    ProposalResultOnly(Box<maestria_domain::ModelAgentProposalRequest>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EffectAdmission {
    Execute {
        risk: RiskClass,
        claim: Option<ApprovedProposalClaim>,
    },
    AwaitingApproval {
        risk: RiskClass,
        reason: String,
        wait: ApprovalWait,
    },
    Rejected {
        risk: RiskClass,
        cause: RejectionCause,
        handling: RejectionHandling,
    },
}

impl EffectExecutionContext {
    fn exact_journal_entry(
        &self,
        proposal: &maestria_domain::ModelAgentProposalRequest,
        generation: maestria_domain::JournalGeneration,
    ) -> Result<Option<EffectJournalEntry>, String> {
        let entries = self
            .adapters
            .effect_journal
            .scan_in_flight()
            .map_err(|error| format!("unable to scan proposal journal: {error}"))?;
        Ok(entries.into_iter().find(|entry| {
            entry.generation == generation.value()
                && journal_entry_matches_proposal(entry, proposal, self.scope_id)
        }))
    }

    fn rejected(risk: RiskClass, reason: impl Into<String>) -> EffectAdmission {
        EffectAdmission::Rejected {
            risk,
            cause: RejectionCause::Reason(reason.into()),
            handling: RejectionHandling::ObserveOnly,
        }
    }

    fn canonical_fresh_proposal_matches(
        &self,
        proposal: &maestria_domain::ModelAgentProposalRequest,
    ) -> Result<(), String> {
        let state = self
            .state
            .try_read()
            .map_err(|_| "model-agent canonical request state is unavailable".to_string())?;
        let Some(canonical) = state.model_agent_requests.get(&proposal.run_id) else {
            return Err("model-agent canonical request is missing".to_string());
        };
        if !matches!(canonical.execution, ModelAgentProposalExecution::Fresh) {
            return Err("model-agent canonical request is not fresh".to_string());
        }
        let mut canonicalized = proposal.clone();
        canonicalized.execution = ModelAgentProposalExecution::Fresh;
        if canonicalized != *canonical {
            return Err(
                "model-agent proposal does not match its canonical fresh request".to_string(),
            );
        }
        Ok(())
    }

    fn admit_journal_recovery(
        &self,
        risk: RiskClass,
        proposal: &maestria_domain::ModelAgentProposalRequest,
        generation: maestria_domain::JournalGeneration,
    ) -> EffectAdmission {
        let entry = match self.exact_journal_entry(proposal, generation) {
            Ok(entry) => entry,
            Err(error) => return Self::rejected(risk, error),
        };
        let Some(entry) = entry else {
            return Self::rejected(
                risk,
                "model-agent journal recovery does not match an exact journal entry",
            );
        };
        if let Err(error) = self.canonical_fresh_proposal_matches(proposal) {
            return Self::rejected(risk, error);
        }
        if entry.status != EffectJournalStatus::FeedbackAccepted || entry.feedback.is_none() {
            return Self::rejected(
                risk,
                "model-agent journal recovery requires durable accepted feedback",
            );
        }
        EffectAdmission::Execute { risk, claim: None }
    }

    fn admit_approval_continuation(
        &self,
        risk: RiskClass,
        proposal: &maestria_domain::ModelAgentProposalRequest,
        approval_id: maestria_domain::ApprovalId,
        generation: maestria_domain::JournalGeneration,
    ) -> EffectAdmission {
        let record = match self.adapters.approval_repo.find_by_id(approval_id) {
            Ok(Some(record)) => record,
            Ok(None) => {
                return Self::rejected(risk, "model-agent proposal approval record not found");
            }
            Err(error) => {
                return EffectAdmission::Rejected {
                    risk,
                    cause: RejectionCause::ApprovalLookup(error),
                    handling: RejectionHandling::ObserveOnly,
                };
            }
        };
        let stored_proposal = match decode_pending_continuation(&record) {
            Ok(Some(proposal)) => proposal,
            Ok(None) | Err(_) => {
                return Self::rejected(
                    risk,
                    "model-agent proposal approval continuation is malformed",
                );
            }
        };
        let identity_matches = record.id == approval_id
            && record.effect_kind == "model_agent_harness"
            && record.scope_id == self.scope_id
            && record.task_id == proposal.task_id
            && record.capability.starts_with("model_agent_pending:")
            && stored_proposal == *proposal;
        if !identity_matches {
            return Self::rejected(
                risk,
                "model-agent proposal does not match its stored approval",
            );
        }
        if let Err(error) = self.canonical_fresh_proposal_matches(proposal) {
            return Self::rejected(risk, error);
        }
        let entry = match self.exact_journal_entry(proposal, generation) {
            Ok(entry) => entry,
            Err(error) => return Self::rejected(risk, error),
        };
        if entry.is_none() && record.status == ApprovalStatus::Denied {
            return EffectAdmission::Rejected {
                risk,
                cause: RejectionCause::Reason("model-agent proposal approval denied".to_string()),
                handling: RejectionHandling::ProposalResultOnly(Box::new(stored_proposal)),
            };
        }
        let Some(entry) = entry else {
            return Self::rejected(
                risk,
                "model-agent approval continuation journal entry is missing",
            );
        };
        if record.status == ApprovalStatus::Denied && entry.status == EffectJournalStatus::Failed {
            return EffectAdmission::Rejected {
                risk,
                cause: RejectionCause::Reason("model-agent proposal approval denied".to_string()),
                handling: RejectionHandling::ProposalResultOnly(Box::new(stored_proposal)),
            };
        }
        if entry.status != EffectJournalStatus::Intent {
            return Self::rejected(
                risk,
                "model-agent approval continuation journal entry is not an exact intent",
            );
        }
        match record.status {
            ApprovalStatus::Approved => EffectAdmission::Execute {
                risk,
                claim: Some(ApprovedProposalClaim {
                    run_id: proposal.run_id,
                    generation: generation.value(),
                }),
            },
            ApprovalStatus::Pending => EffectAdmission::AwaitingApproval {
                risk,
                reason: "model-agent proposal approval is still pending".to_string(),
                wait: ApprovalWait::ExistingProposalApproval,
            },
            ApprovalStatus::Denied => EffectAdmission::Rejected {
                risk,
                cause: RejectionCause::Reason("model-agent proposal approval denied".to_string()),
                handling: RejectionHandling::Proposal(Box::new(stored_proposal)),
            },
        }
    }

    fn admit_fresh_proposal(
        &self,
        effect: &MaestriaEffect,
        risk: RiskClass,
        proposal: &maestria_domain::ModelAgentProposalRequest,
    ) -> EffectAdmission {
        let scope = ScopeGuard::new(self.scope.clone());
        let decision = self
            .governance
            .approval_gate
            .decide(&ApprovalRequest {
                effect,
                profile: self.profile,
                scope: &scope,
                risk,
            })
            .decision;
        match decision {
            PolicyDecision::Allow => EffectAdmission::Execute { risk, claim: None },
            PolicyDecision::Deny { reason } => EffectAdmission::Rejected {
                risk,
                cause: RejectionCause::Reason(reason),
                handling: RejectionHandling::ProposalResultOnly(Box::new(proposal.clone())),
            },
            PolicyDecision::RequireApproval { reason } => EffectAdmission::AwaitingApproval {
                risk,
                reason,
                wait: ApprovalWait::CreateProposalApproval,
            },
        }
    }

    pub(crate) fn admit_effect(&self, effect: &MaestriaEffect) -> EffectAdmission {
        let scope = ScopeGuard::new(self.scope.clone());
        let risk = self.governance.classifier.classify(effect, &scope);

        if matches!(effect, MaestriaEffect::RequestApproval(_)) {
            return EffectAdmission::Execute { risk, claim: None };
        }

        if let MaestriaEffect::QueryHarnessProposal(request) = effect {
            return match &request.proposal.execution {
                ModelAgentProposalExecution::Fresh => {
                    self.admit_fresh_proposal(effect, risk, &request.proposal)
                }
                ModelAgentProposalExecution::JournalRecovery { journal_generation } => {
                    self.admit_journal_recovery(risk, &request.proposal, *journal_generation)
                }
                ModelAgentProposalExecution::ApprovalContinuation {
                    approval_id,
                    journal_generation,
                } => self.admit_approval_continuation(
                    risk,
                    &request.proposal,
                    *approval_id,
                    *journal_generation,
                ),
            };
        }

        let decision = self
            .governance
            .approval_gate
            .decide(&ApprovalRequest {
                effect,
                profile: self.profile,
                scope: &scope,
                risk,
            })
            .decision;
        match decision {
            PolicyDecision::Allow => EffectAdmission::Execute { risk, claim: None },
            PolicyDecision::Deny { reason } => EffectAdmission::Rejected {
                risk,
                cause: RejectionCause::Reason(reason),
                handling: match effect {
                    MaestriaEffect::QueryHarness(request) => {
                        RejectionHandling::LegacyHarness(Box::new(request.clone()))
                    }
                    _ => RejectionHandling::ObserveOnly,
                },
            },
            PolicyDecision::RequireApproval { reason } => EffectAdmission::Rejected {
                risk,
                cause: RejectionCause::Reason(format!(
                    "effect has no durable approval continuation; policy reason: {reason}"
                )),
                handling: match effect {
                    MaestriaEffect::QueryHarness(request) => {
                        RejectionHandling::LegacyHarness(Box::new(request.clone()))
                    }
                    _ => RejectionHandling::ObserveOnly,
                },
            },
        }
    }
}
