use crate::config::EffectExecutionContext;
use crate::effect_execution::{
    decode_pending_continuation, persist_pending_harness, record_denied_harness,
    resume_harness_journal,
};
use crate::effect_result::{EffectFailure, handler_result};
use maestria_domain::MaestriaEffect;
use maestria_governance::{ApprovalRequest, PolicyDecision, RiskClass, ScopeGuard};
use maestria_ports::{ApprovalStatus, EffectJournalIntent, EffectJournalStatus};
use std::time::Duration;

impl EffectExecutionContext {
    fn classify_effect(&self, effect: &MaestriaEffect) -> (RiskClass, PolicyDecision) {
        let scope = ScopeGuard::new(self.scope.clone());
        let risk = self.governance.classifier.classify(effect, &scope);
        let proposal_approval = match effect {
            MaestriaEffect::QueryHarnessProposal(request) => {
                request.proposal.approval_id.and_then(|approval_id| {
                    self.adapters
                        .approval_repo
                        .find_by_id(approval_id)
                        .ok()
                        .flatten()
                        .map(|record| {
                            let identity_matches = decode_pending_continuation(&record).as_ref()
                                == Some(&request.proposal);
                            (record.status, identity_matches)
                        })
                })
            }
            _ => None,
        };
        let decision = match proposal_approval {
            Some((ApprovalStatus::Approved, true)) => PolicyDecision::Allow,
            Some((ApprovalStatus::Denied, true)) => PolicyDecision::Deny {
                reason: "model-agent proposal approval denied".to_string(),
            },
            Some((_, false)) => PolicyDecision::Deny {
                reason: "model-agent proposal does not match its stored approval".to_string(),
            },
            _ => {
                self.governance
                    .approval_gate
                    .decide(&ApprovalRequest {
                        effect,
                        profile: self.profile,
                        scope: &scope,
                        risk,
                    })
                    .decision
            }
        };
        (risk, decision)
    }

    async fn enforce_effect_policy(
        &self,
        effect: &MaestriaEffect,
        risk: RiskClass,
        decision: PolicyDecision,
    ) -> Result<(), EffectFailure> {
        match decision {
            PolicyDecision::Allow => Ok(()),
            PolicyDecision::Deny { reason } => {
                tracing::warn!(?risk, %reason, "effect denied");
                match effect {
                    MaestriaEffect::QueryHarness(request) => {
                        record_denied_harness(self, request)?;
                    }
                    MaestriaEffect::QueryHarnessProposal(request) => {
                        let generation =
                            if let Some(generation) = request.proposal.journal_generation {
                                generation
                            } else {
                                let entry = self
                                    .adapters
                                    .effect_journal
                                    .record_intent(EffectJournalIntent {
                                        run_id: request.proposal.run_id,
                                        task_id: request.proposal.task_id,
                                        capability: request.proposal.capability.clone(),
                                        command: request.proposal.command.clone(),
                                        scope_id: self.scope_id,
                                        requested_generation: None,
                                    })
                                    .map_err(|error| {
                                        EffectFailure::Failed(format!(
                                            "record denied proposal intent: {error}"
                                        ))
                                    })?;
                                entry.generation
                            };
                        self.adapters
                            .effect_journal
                            .record_terminal(
                                request.proposal.run_id,
                                generation,
                                EffectJournalStatus::Failed,
                            )
                            .map_err(|error| {
                                EffectFailure::Failed(format!(
                                    "record denied proposal terminal: {error}"
                                ))
                            })?;
                        self.record_model_agent_denial(&request.proposal, reason.clone())
                            .await?;
                    }
                    _ => {}
                }
                Err(EffectFailure::Denied(reason))
            }
            PolicyDecision::RequireApproval { reason } => {
                tracing::info!(?risk, %reason, "effect requires approval");
                if let MaestriaEffect::QueryHarnessProposal(request) = effect {
                    if request.proposal.approval_id.is_none() {
                        persist_pending_harness(self, request).await?;
                    } else {
                        resume_harness_journal(self, &request.proposal)?;
                    }
                } else if let MaestriaEffect::QueryHarness(request) = effect {
                    // Legacy harness requests have no resumable proposal payload.
                    record_denied_harness(self, request)?;
                }
                Err(EffectFailure::RequiresApproval(reason))
            }
        }
    }

    async fn dispatch_effect(
        self,
        effect: MaestriaEffect,
        persistence_barrier_timeout: Option<Duration>,
    ) -> Result<(), EffectFailure> {
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

    /// Execute a single effect after governance classification.
    pub(crate) async fn execute_effect(
        self,
        effect: MaestriaEffect,
        persistence_barrier_timeout: Option<Duration>,
    ) -> Result<(), EffectFailure> {
        let (risk, decision) = self.classify_effect(&effect);
        self.enforce_effect_policy(&effect, risk, decision).await?;
        self.dispatch_effect(effect, persistence_barrier_timeout)
            .await
    }
}
