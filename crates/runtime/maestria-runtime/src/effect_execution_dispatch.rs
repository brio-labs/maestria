use crate::config::EffectExecutionContext;
use crate::effect_admission::{
    ApprovalWait, ApprovedProposalClaim, EffectAdmission, RejectionCause, RejectionHandling,
};
use crate::effect_execution::{persist_pending_harness, record_denied_harness};
use crate::effect_result::{EffectFailure, handler_result};
use crate::proposal_recovery::journal_entry_matches_proposal;
use maestria_domain::{MaestriaEffect, ModelAgentProposalExecution};
use maestria_governance::RiskClass;
use maestria_ports::EffectJournalStatus;
use std::collections::BTreeSet;
use std::time::Duration;

impl EffectExecutionContext {
    fn claim_approved_proposal(&self, claim: ApprovedProposalClaim) -> Result<(), EffectFailure> {
        match self
            .adapters
            .effect_journal
            .record_started(claim.run_id, claim.generation)
        {
            Ok(()) => Ok(()),
            Err(maestria_ports::PortError::NotFound) => Err(EffectFailure::Denied(
                "approved proposal journal intent was already claimed or is unavailable"
                    .to_string(),
            )),
            Err(error) => Err(EffectFailure::Failed(format!(
                "claim approved proposal journal intent: {error}"
            ))),
        }
    }

    fn claim_journal_recovery(
        &self,
        proposal: &maestria_domain::ModelAgentProposalRequest,
        generation: u64,
    ) -> Result<(), EffectFailure> {
        let mut claims = self.journal_recovery_claims.lock().map_err(|_| {
            EffectFailure::Failed("journal recovery claim lock poisoned".to_string())
        })?;
        let entries = self
            .adapters
            .effect_journal
            .scan_in_flight()
            .map_err(|error| {
                EffectFailure::Failed(format!("scan journal recovery claims: {error}"))
            })?;
        let active_keys: BTreeSet<_> = entries
            .iter()
            .filter(|entry| {
                entry.status == EffectJournalStatus::FeedbackAccepted && entry.feedback.is_some()
            })
            .map(|entry| (entry.run_id, entry.generation))
            .collect();
        let key = (proposal.run_id, generation);
        let exact_active_entry = entries.iter().any(|entry| {
            entry.generation == generation
                && journal_entry_matches_proposal(entry, proposal, self.scope_id)
                && entry.status == EffectJournalStatus::FeedbackAccepted
                && entry.feedback.is_some()
        });
        if !exact_active_entry {
            return Err(EffectFailure::Denied(
                "model-agent journal recovery has no exact accepted feedback entry".to_string(),
            ));
        }
        claims.retain(|claimed| active_keys.contains(claimed));
        if !claims.insert(key) {
            return Err(EffectFailure::Denied(
                "model-agent journal recovery was already claimed".to_string(),
            ));
        }
        Ok(())
    }

    async fn reject_effect(
        &self,
        risk: RiskClass,
        cause: RejectionCause,
        handling: RejectionHandling,
    ) -> Result<(), EffectFailure> {
        let reason = match cause {
            RejectionCause::Reason(reason) => reason,
            RejectionCause::ApprovalLookup(error) => {
                tracing::error!(%error, "effect approval lookup failed during admission");
                return Err(EffectFailure::ApprovalLookup(error));
            }
        };
        tracing::warn!(?risk, reason = %reason, "effect rejected");
        match handling {
            RejectionHandling::ObserveOnly => {}
            RejectionHandling::LegacyHarness(request) => {
                record_denied_harness(self, &request)?;
            }
            RejectionHandling::ProposalResultOnly(proposal) => {
                self.record_model_agent_denial(&proposal, reason.clone())
                    .await?;
            }
            RejectionHandling::Proposal(proposal) => {
                let ModelAgentProposalExecution::ApprovalContinuation {
                    journal_generation, ..
                } = &proposal.execution
                else {
                    return Err(EffectFailure::Failed(
                        "stored proposal denial lacks an approval continuation".to_string(),
                    ));
                };
                self.adapters
                    .effect_journal
                    .record_terminal(
                        proposal.run_id,
                        *journal_generation,
                        maestria_ports::EffectJournalStatus::Failed,
                    )
                    .map_err(|error| {
                        EffectFailure::Failed(format!("record denied proposal terminal: {error}"))
                    })?;
                self.record_model_agent_denial(&proposal, reason.clone())
                    .await?;
            }
        }
        Err(EffectFailure::Denied(reason))
    }

    async fn await_effect(
        &self,
        effect: &MaestriaEffect,
        risk: RiskClass,
        reason: String,
        wait: ApprovalWait,
    ) -> Result<(), EffectFailure> {
        tracing::info!(?risk, reason = %reason, ?wait, "effect requires approval");
        match wait {
            ApprovalWait::CreateProposalApproval => {
                let MaestriaEffect::QueryHarnessProposal(request) = effect else {
                    return Err(EffectFailure::Denied(
                        "approval continuation is not a model-agent proposal".to_string(),
                    ));
                };
                persist_pending_harness(self, request).await?;
                Err(EffectFailure::RequiresApproval(reason))
            }
            ApprovalWait::ExistingProposalApproval => {
                if !matches!(effect, MaestriaEffect::QueryHarnessProposal(_)) {
                    return Err(EffectFailure::Denied(
                        "existing approval continuation is not a model-agent proposal".to_string(),
                    ));
                }
                Err(EffectFailure::RequiresApproval(reason))
            }
        }
    }
}

impl EffectExecutionContext {
    async fn dispatch_effect(
        self,
        effect: MaestriaEffect,
        risk: RiskClass,
        persistence_barrier_timeout: Option<Duration>,
    ) -> Result<(), EffectFailure> {
        tracing::debug!(?risk, "dispatching admitted effect");
        match effect {
            MaestriaEffect::PersistEvent { envelope } => {
                handler_result(self.handle_persist_event(*envelope).await, "persist event")
            }
            MaestriaEffect::ParseArtifact(request) => handler_result(
                self.handle_parse_artifact(request, persistence_barrier_timeout)
                    .await,
                "parse artifact",
            ),
            MaestriaEffect::Ocr(effect) => {
                handler_result(self.handle_ocr(effect).await, "OCR execution")
            }
            MaestriaEffect::IndexFullText(request) => handler_result(
                self.handle_index_full_text(request).await,
                "index full text",
            ),
            MaestriaEffect::IndexVector(request) => self.handle_index_vector(request).await,
            MaestriaEffect::UpdateGraph(request) => {
                handler_result(self.handle_update_graph(request).await, "update graph")
            }
            MaestriaEffect::QueryHarness(request) => self.handle_query_harness(request).await,
            MaestriaEffect::QueryHarnessProposal(request) => {
                self.handle_query_harness_proposal(request).await
            }
            MaestriaEffect::FetchWeb(request) => {
                handler_result(self.handle_fetch_web(request).await, "fetch web")
            }
            MaestriaEffect::RunValidation(request) => {
                handler_result(self.handle_run_validation(request).await, "run validation")
            }
            MaestriaEffect::RequestApproval(request) => handler_result(
                self.handle_request_approval(request).await,
                "request approval",
            ),
            MaestriaEffect::EmitDiagnostic(diagnostic) => handler_result(
                self.handle_emit_diagnostic(diagnostic).await,
                "emit diagnostic",
            ),
            MaestriaEffect::SearchKnowledge(request) => handler_result(
                self.handle_search_knowledge(*request).await,
                "search knowledge",
            ),
        }
    }

    pub(crate) async fn execute_effect(
        self,
        effect: MaestriaEffect,
        persistence_barrier_timeout: Option<Duration>,
    ) -> Result<(), EffectFailure> {
        let admission = self.admit_effect(&effect);
        match admission {
            EffectAdmission::Execute { risk, claim } => {
                if let MaestriaEffect::QueryHarnessProposal(request) = &effect
                    && let ModelAgentProposalExecution::JournalRecovery { journal_generation } =
                        request.proposal.execution
                {
                    self.claim_journal_recovery(&request.proposal, journal_generation)?;
                }
                if let Some(claim) = claim {
                    self.claim_approved_proposal(claim)?;
                }
                self.dispatch_effect(effect, risk, persistence_barrier_timeout)
                    .await
            }
            EffectAdmission::AwaitingApproval { risk, reason, wait } => {
                self.await_effect(&effect, risk, reason, wait).await
            }
            EffectAdmission::Rejected {
                risk,
                cause,
                handling,
            } => self.reject_effect(risk, cause, handling).await,
        }
    }
}
