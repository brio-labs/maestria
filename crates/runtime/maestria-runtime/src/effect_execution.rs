use crate::config::EffectExecutionContext;
use crate::effect_result::{EffectFailure, handler_result};
use maestria_domain::{
    CorpusScope, DiagnosticEvent, DomainInput, LogicalTick, MaestriaEffect, RequestApprovalRequest,
    SearchKnowledgeCompleted, SearchKnowledgeRequest, UpdateGraphRequest,
};
use maestria_governance::{ApprovalRequest, PolicyDecision, RiskClass, ScopeGuard};
use maestria_ports::{ApprovalRecord, ApprovalRiskLevel, ApprovalStatus};
use std::time::Duration;
use tokio::sync::mpsc;

// ── dispatch ──────────────────────────────────────────────────────────

impl EffectExecutionContext {
    /// Execute a single effect after governance classification.
    /// Persistence variants delegate to the persistence module;
    /// heavy handlers (parsing, indexing, harness, validation) live
    /// in their own modules to keep focused responsibility boundaries.
    pub(crate) async fn execute_effect(
        self,
        effect: MaestriaEffect,
        persistence_barrier_timeout: Option<Duration>,
    ) -> Result<(), EffectFailure> {
        let scope = ScopeGuard::new(self.scope.clone());
        let risk = self.governance.classifier.classify(&effect, &scope);
        let decision = self.governance.approval_gate.decide(&ApprovalRequest {
            effect: &effect,
            profile: self.profile,
            scope: &scope,
        });

        match decision.decision {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => {
                tracing::warn!(?risk, %reason, "effect denied");
                return Err(EffectFailure::Denied(reason));
            }
            PolicyDecision::RequireApproval { reason } => {
                tracing::info!(?risk, %reason, "effect requires approval");
                return Err(EffectFailure::RequiresApproval(reason));
            }
        }

        match effect {
            MaestriaEffect::PersistEvent { envelope } => {
                handler_result(self.handle_persist_event(*envelope).await, "persist event")
            }
            MaestriaEffect::PersistState(request) => {
                handler_result(self.handle_persist_state(request).await, "persist state")
            }
            MaestriaEffect::ParseArtifact(request) => handler_result(
                self.handle_parse_artifact(request, persistence_barrier_timeout)
                    .await,
                "parse artifact",
            ),
            MaestriaEffect::IndexFullText(request) => handler_result(
                self.handle_index_full_text(request).await,
                "index full text",
            ),
            MaestriaEffect::IndexVector(request) => self.handle_index_vector(request).await,
            MaestriaEffect::UpdateGraph(request) => {
                handler_result(self.handle_update_graph(request).await, "update graph")
            }
            MaestriaEffect::QueryHarness(request) => {
                handler_result(self.handle_query_harness(request).await, "query harness")
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

    /// Retry loop with timeout watchdog. Non-idempotent harness effects never
    /// replay automatically; their journal entry pauses or fails instead.
    pub(crate) async fn execute_with_retries(self, effect: MaestriaEffect) -> Result<(), EffectFailure> {
        let non_idempotent = matches!(&effect, MaestriaEffect::QueryHarness(_));
        let watchdog = self.default_effect_timeout + Duration::from_secs(5);
        let result = tokio::time::timeout(watchdog, async {
            let mut attempts = 0;
            loop {
                match self
                    .clone()
                    .execute_effect(effect.clone(), Some(self.default_effect_timeout))
                    .await
                {
                    Ok(()) => return Ok(()),
                    Err(error) => {
                        tracing::error!(%error, "effect execution did not complete");
                        if !error.retryable() || non_idempotent || attempts >= self.max_retries {
                            return Err(error);
                        }
                    }
                }
                attempts += 1;
                tracing::warn!("Retrying effect execution (attempt {})", attempts);
                tokio::time::sleep(Duration::from_millis(500 * (1 << attempts))).await;
            }
        })
        .await;

        match result {
            Ok(inner) => inner,
            Err(_) => {
                tracing::error!(
                    "Watchdog: effect execution timed out after {:?}",
                    watchdog
                );
                Err(EffectFailure::Failed("effect watchdog timeout".to_string()))
            }
        }
    }

    /// Fire-and-forget send into the domain input channel.
    /// Logs failures but never propagates them — the runtime loop
    /// already has a shutdown path for backpressure.
    pub(crate) fn send_input(
        input_tx: &mpsc::Sender<DomainInput>,
        input: DomainInput,
        context: &'static str,
    ) -> Result<(), crate::FeedbackError> {
        input_tx.try_send(input).map_err(|e| {
            tracing::error!(error = %e, context, "failed to send domain input (backpressure)");
            match e {
                mpsc::error::TrySendError::Full(_) => crate::FeedbackError::CapacityFull,
                mpsc::error::TrySendError::Closed(_) => crate::FeedbackError::RuntimeShutdown,
            }
        })
    }

    // ── lightweight handlers ──────────────────────────────────────────

    async fn handle_search_knowledge(&self, request: SearchKnowledgeRequest) -> bool {
        let Some(executor) = &self.adapters.search_executor else {
            tracing::error!("search knowledge effect has no configured executor");
            return false;
        };
        let mut plan = request.plan;
        match &mut plan.scope {
            CorpusScope::Global => {
                plan.scope = CorpusScope::Restricted(vec![self.scope_id]);
            }
            CorpusScope::Restricted(scopes) if scopes.as_slice() != [self.scope_id] => {
                tracing::error!("search knowledge request exceeds runtime scope");
                return false;
            }
            CorpusScope::Restricted(_) => {}
        }
        match executor.search(plan.clone()).await {
            Ok(outcome) => {
                if let Err(error) = outcome.verify_compatibility(&plan) {
                    tracing::error!(%error, "search outcome is incompatible with request plan");
                    return false;
                }
                Self::send_input(
                    &self.input_tx,
                    DomainInput::SearchKnowledgeCompleted(SearchKnowledgeCompleted {
                        task_id: request.task_id,
                        plan: Some(Box::new(plan)),
                        outcome,
                    }),
                    "search knowledge completion",
                )
                .is_ok()
            }
            Err(error) => {
                tracing::error!(%error, "knowledge search failed");
                false
            }
        }
    }

    async fn handle_update_graph(&self, request: UpdateGraphRequest) -> bool {
        let relation = {
            let state = self.state.read().await;
            state.relations.get(&request.relation_id).cloned()
        };
        let Some(relation) = relation else {
            tracing::error!(relation_id = %request.relation_id, "relation missing for graph update");
            return false;
        };
        if relation.evidence_id.is_none() {
            tracing::warn!(
                relation_id = %request.relation_id,
                "refusing to project unevidenced relation"
            );
            return false;
        }
        if let Err(error) = self.adapters.graph_index.insert_relation(relation) {
            tracing::error!(relation_id = %request.relation_id, %error, "failed to insert relation into graph");
            return false;
        }
        true
    }

    async fn handle_request_approval(&self, request: RequestApprovalRequest) -> bool {
        let approval_id = match self.adapters.id_allocator.allocate_approval_id() {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(%e, "failed to allocate approval id");
                return false;
            }
        };

        // Compute risk using the governance classifier.
        let scope_guard = ScopeGuard::new(self.scope.clone());
        let effect = MaestriaEffect::RequestApproval(RequestApprovalRequest {
            task_id: request.task_id,
        });
        let risk = self.governance.classifier.classify(&effect, &scope_guard);
        let risk_level = risk_class_to_approval_risk_level(risk);

        let tick = {
            let state = self.state.read().await;
            match state.event_log.last() {
                Some(e) => LogicalTick::new(e.sequence.value()),
                None => LogicalTick::new(0),
            }
        };

        let record = ApprovalRecord {
            id: approval_id,
            task_id: request.task_id,
            effect_kind: "task_activation".to_string(),
            risk_level,
            capability: "task_activation".to_string(),
            scope_id: self.scope_id,
            tick,
            status: ApprovalStatus::Pending,
        };

        if let Err(e) = self.adapters.approval_repo.save(&record) {
            tracing::error!(%e, approval_id=%approval_id, "failed to persist approval request");
            return false;
        }

        tracing::info!(
            approval_id = %approval_id,
            task_id = %request.task_id,
            "approval request persisted; awaiting external resolution"
        );
        true
    }

    async fn handle_emit_diagnostic(&self, diagnostic: DiagnosticEvent) -> bool {
        tracing::info!(
            task_id = ?diagnostic.task_id,
            message = %diagnostic.message,
            "domain diagnostic"
        );
        true
    }
}

fn risk_class_to_approval_risk_level(risk: RiskClass) -> ApprovalRiskLevel {
    match risk {
        RiskClass::Low => ApprovalRiskLevel::Low,
        RiskClass::Medium => ApprovalRiskLevel::Medium,
        RiskClass::High => ApprovalRiskLevel::High,
        RiskClass::Critical => ApprovalRiskLevel::Critical,
    }
}
