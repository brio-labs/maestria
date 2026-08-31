use crate::config::EffectExecutionContext;
use crate::effect_admission::{
    ApprovalWait, ApprovedProposalClaim, EffectAdmission, RejectionCause, RejectionHandling,
};
use crate::effect_result::{EffectFailure, handler_result};
use crate::proposal_persistence::{persist_pending_harness, record_denied_harness};
use crate::proposal_recovery::journal_entry_matches_proposal;
use crate::proposal_workflow::model_agent_denial_result;
use maestria_domain::{MaestriaEffect, ModelAgentProposalExecution};
use maestria_governance::RiskClass;
use maestria_ports::EffectJournalStatus;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

/// Static counter for throttled effect-denial logging: (effect name,
/// reason) → occurrences. Identical denials are logged on the first
/// occurrence and every 100th thereafter.
static DENIAL_LOG_COUNTS: Mutex<BTreeMap<(&'static str, String), usize>> =
    Mutex::new(BTreeMap::new());

/// The stable variant name of an effect, for logs.
fn effect_variant_name(effect: &MaestriaEffect) -> &'static str {
    match effect {
        MaestriaEffect::PersistEvent { .. } => "persist_event",
        MaestriaEffect::PersistNotebookDraftBlob(_) => "persist_notebook_draft_blob",
        MaestriaEffect::ParseArtifact(_) => "parse_artifact",
        MaestriaEffect::Ocr(_) => "ocr",
        MaestriaEffect::IndexFullText(_) => "index_full_text",
        MaestriaEffect::IndexArtifactVectors(_) => "index_artifact_vectors",
        MaestriaEffect::UpdateGraph(_) => "update_graph",
        MaestriaEffect::QueryHarness(_) => "query_harness",
        MaestriaEffect::QueryHarnessProposal(_) => "query_harness_proposal",
        MaestriaEffect::FetchWeb(_) => "fetch_web",
        MaestriaEffect::RunValidation(_) => "run_validation",
        MaestriaEffect::RequestApproval(_) => "request_approval",
        MaestriaEffect::SearchKnowledge(_) => "search_knowledge",
    }
}

pub(crate) enum PreparedEffect {
    Dispatch {
        effect: Box<MaestriaEffect>,
        risk: RiskClass,
    },
    TerminalResult(Box<maestria_domain::ModelAgentProposalResult>),
}

impl EffectExecutionContext {
    fn claim_approved_proposal(&self, claim: ApprovedProposalClaim) -> Result<(), EffectFailure> {
        match self.adapters.effect_journal.record_started(
            claim.run_id,
            maestria_domain::JournalGeneration::new(claim.generation),
        ) {
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
        generation: maestria_domain::JournalGeneration,
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
        effect: &MaestriaEffect,
        risk: RiskClass,
        cause: RejectionCause,
        handling: RejectionHandling,
    ) -> EffectFailure {
        let reason = match cause {
            RejectionCause::Reason(reason) => reason,
            RejectionCause::ApprovalLookup(error) => {
                tracing::error!(%error, "effect approval lookup failed during admission");
                return EffectFailure::ApprovalLookup(error);
            }
        };
        // Throttle identical denials: the first occurrence logs in full,
        // then every 100th with the running count. The domain can emit a
        // denied effect per chunk or per evidence (provider-less medium
        // effects), which would otherwise flood the log at home scale.
        let name = effect_variant_name(effect);
        let key = (name, reason.clone());
        let count = {
            let mut counts = match DENIAL_LOG_COUNTS.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let count = counts.entry(key).or_insert(0);
            *count += 1;
            *count
        };
        if count == 1 || count.is_multiple_of(100) {
            tracing::warn!(effect = %name, ?risk, reason = %reason, count, "effect rejected");
        }
        match handling {
            RejectionHandling::ObserveOnly => {}
            RejectionHandling::LegacyHarness(request) => {
                if let Err(error) = record_denied_harness(self, &request) {
                    return error;
                }
            }
            RejectionHandling::ProposalResultOnly(proposal) => {
                if let Err(error) = self
                    .record_model_agent_denial(&proposal, reason.clone())
                    .await
                {
                    return error;
                }
            }
            RejectionHandling::Proposal(proposal) => {
                let ModelAgentProposalExecution::ApprovalContinuation {
                    journal_generation, ..
                } = &proposal.execution
                else {
                    return EffectFailure::Failed(
                        "stored proposal denial lacks an approval continuation".to_string(),
                    );
                };
                if let Err(error) = self
                    .adapters
                    .effect_journal
                    .record_terminal(
                        proposal.run_id,
                        *journal_generation,
                        maestria_ports::EffectJournalStatus::Failed,
                    )
                    .map_err(|error| {
                        EffectFailure::Failed(format!("record denied proposal terminal: {error}"))
                    })
                {
                    return error;
                }
                if let Err(error) = self
                    .record_model_agent_denial(&proposal, reason.clone())
                    .await
                {
                    return error;
                }
            }
        }
        EffectFailure::Denied(reason)
    }

    async fn await_effect(
        &self,
        effect: &MaestriaEffect,
        risk: RiskClass,
        reason: String,
        wait: ApprovalWait,
    ) -> EffectFailure {
        tracing::info!(?risk, reason = %reason, ?wait, "effect requires approval");
        match wait {
            ApprovalWait::CreateProposalApproval => {
                let MaestriaEffect::QueryHarnessProposal(request) = effect else {
                    return EffectFailure::Denied(
                        "approval continuation is not a model-agent proposal".to_string(),
                    );
                };
                match persist_pending_harness(self, request).await {
                    Ok(()) => EffectFailure::RequiresApproval(reason),
                    Err(error) => error,
                }
            }
            ApprovalWait::ExistingProposalApproval => {
                if matches!(effect, MaestriaEffect::QueryHarnessProposal(_)) {
                    EffectFailure::RequiresApproval(reason)
                } else {
                    EffectFailure::Denied(
                        "existing approval continuation is not a model-agent proposal".to_string(),
                    )
                }
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
            MaestriaEffect::PersistNotebookDraftBlob(request) => handler_result(
                self.handle_persist_notebook_draft_blob(request).await,
                "persist notebook draft blob",
            ),
            MaestriaEffect::ParseArtifact(request) => handler_result(
                self.handle_parse_artifact(request, persistence_barrier_timeout)
                    .await,
                "parse artifact",
            ),
            MaestriaEffect::Ocr(effect) => self.handle_ocr(effect).await,
            MaestriaEffect::IndexFullText(request) => handler_result(
                self.handle_index_full_text(request).await,
                "index full text",
            ),
            MaestriaEffect::IndexArtifactVectors(request) => {
                self.handle_index_artifact_vectors(request).await
            }
            MaestriaEffect::UpdateGraph(request) => {
                handler_result(self.handle_update_graph(request).await, "update graph")
            }
            MaestriaEffect::QueryHarness(request) => self.handle_query_harness(request).await,
            MaestriaEffect::QueryHarnessProposal(request) => {
                self.handle_query_harness_proposal(*request).await
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
            MaestriaEffect::SearchKnowledge(request) => handler_result(
                self.handle_search_knowledge(*request).await,
                "search knowledge",
            ),
        }
    }

    fn prepare_terminal_rejection(
        &self,
        risk: RiskClass,
        cause: RejectionCause,
        handling: RejectionHandling,
    ) -> Result<PreparedEffect, EffectFailure> {
        let reason = match cause {
            RejectionCause::Reason(reason) => reason,
            RejectionCause::ApprovalLookup(error) => {
                return Err(EffectFailure::ApprovalLookup(error));
            }
        };
        tracing::warn!(?risk, reason = %reason, "effect rejected");
        let proposal = match handling {
            RejectionHandling::ProposalResultOnly(proposal) => proposal,
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
                proposal
            }
            _ => {
                return Err(EffectFailure::Failed(
                    "terminal rejection does not own a proposal result".to_string(),
                ));
            }
        };
        Ok(PreparedEffect::TerminalResult(Box::new(
            model_agent_denial_result(&proposal, reason),
        )))
    }

    async fn prepare_effect_inner(
        &self,
        effect: MaestriaEffect,
        terminal_denial_is_success: bool,
    ) -> Result<PreparedEffect, EffectFailure> {
        let admission = self.admit_effect(&effect);
        match admission {
            EffectAdmission::Execute { risk, claim } => {
                if let MaestriaEffect::QueryHarnessProposal(request) = &effect
                    && let ModelAgentProposalExecution::JournalRecovery { journal_generation } =
                        request.execution
                {
                    self.claim_journal_recovery(request, journal_generation)?;
                }
                if let Some(claim) = claim {
                    self.claim_approved_proposal(claim)?;
                }
                Ok(PreparedEffect::Dispatch {
                    effect: Box::new(effect),
                    risk,
                })
            }
            EffectAdmission::AwaitingApproval { risk, reason, wait } => {
                Err(self.await_effect(&effect, risk, reason, wait).await)
            }
            EffectAdmission::Rejected {
                risk,
                cause,
                handling,
            } => {
                if terminal_denial_is_success
                    && matches!(
                        &handling,
                        RejectionHandling::Proposal(_) | RejectionHandling::ProposalResultOnly(_)
                    )
                {
                    self.prepare_terminal_rejection(risk, cause, handling)
                } else {
                    Err(self.reject_effect(&effect, risk, cause, handling).await)
                }
            }
        }
    }

    pub(crate) async fn prepare_effect(
        &self,
        effect: MaestriaEffect,
    ) -> Result<PreparedEffect, EffectFailure> {
        self.prepare_effect_inner(effect, false).await
    }

    pub(crate) async fn prepare_effect_before_reply(
        &self,
        effect: MaestriaEffect,
    ) -> Result<PreparedEffect, EffectFailure> {
        self.prepare_effect_inner(effect, true).await
    }

    pub(crate) async fn execute_prepared(
        self,
        prepared: PreparedEffect,
        persistence_barrier_timeout: Option<Duration>,
    ) -> Result<(), EffectFailure> {
        match prepared {
            PreparedEffect::Dispatch { effect, risk } => {
                self.dispatch_effect(*effect, risk, persistence_barrier_timeout)
                    .await
            }
            PreparedEffect::TerminalResult(result) => {
                self.persist_model_agent_result(*result).await
            }
        }
    }

    pub(crate) async fn execute_effect(
        self,
        effect: MaestriaEffect,
        persistence_barrier_timeout: Option<Duration>,
    ) -> Result<(), EffectFailure> {
        let prepared = self.prepare_effect(effect).await?;
        self.execute_prepared(prepared, persistence_barrier_timeout)
            .await
    }
}
