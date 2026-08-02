use crate::config::EffectExecutionContext;
use crate::effect_execution_dispatch::PreparedEffect;
use crate::effect_result::EffectFailure;
use crate::proposal_persistence::risk_class_to_approval_risk_level;
use maestria_domain::{
    DiagnosticEvent, DomainInput, LogicalTick, MaestriaEffect, RequestApprovalRequest,
    SearchKnowledgeCompleted, SearchKnowledgeRequest, UpdateGraphRequest,
};
use maestria_governance::ScopeGuard;
use maestria_ports::{ApprovalRecord, ApprovalStatus};
use std::time::Duration;
use tokio::sync::mpsc;

impl EffectExecutionContext {
    /// Retry loop with timeout watchdog. Non-idempotent harness effects never
    /// replay automatically; their journal entry pauses or fails instead.
    pub(crate) async fn execute_with_retries(
        self,
        effect: MaestriaEffect,
    ) -> Result<(), EffectFailure> {
        let non_idempotent = matches!(
            &effect,
            MaestriaEffect::QueryHarness(_) | MaestriaEffect::QueryHarnessProposal(_)
        );
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
                tracing::error!("Watchdog: effect execution timed out after {:?}", watchdog);
                Err(EffectFailure::Failed("effect watchdog timeout".to_string()))
            }
        }
    }

    pub(crate) async fn execute_prepared_with_watchdog(
        self,
        prepared: PreparedEffect,
    ) -> Result<(), EffectFailure> {
        let watchdog = self.default_effect_timeout + Duration::from_secs(5);
        match tokio::time::timeout(
            watchdog,
            self.clone()
                .execute_prepared(prepared, Some(self.default_effect_timeout)),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                tracing::error!("Watchdog: prepared effect timed out after {:?}", watchdog);
                Err(EffectFailure::Failed(
                    "prepared effect watchdog timeout".to_string(),
                ))
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

    pub(crate) async fn handle_search_knowledge(&self, request: SearchKnowledgeRequest) -> bool {
        let Some(executor) = &self.adapters.search_executor else {
            tracing::error!("search knowledge effect has no configured executor");
            return false;
        };
        let mut plan = request.plan;
        // One owner of search-scope confinement (R28/R43): the same typed
        // transition used by direct CLI/API search surfaces.
        plan = match plan.confine_to_scope(self.scope_id) {
            Ok(confined) => confined,
            Err(error) => {
                tracing::error!(%error, "search knowledge scope confinement rejected");
                return false;
            }
        };
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
                        plan: Box::new(plan),
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

    pub(crate) async fn handle_update_graph(&self, request: UpdateGraphRequest) -> bool {
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

    pub(crate) async fn handle_request_approval(&self, request: RequestApprovalRequest) -> bool {
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
            task_id: Some(request.task_id),
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

    pub(crate) async fn handle_emit_diagnostic(&self, diagnostic: DiagnosticEvent) -> bool {
        tracing::info!(
            task_id = ?diagnostic.task_id,
            message = %diagnostic.message,
            "domain diagnostic"
        );
        true
    }
}
