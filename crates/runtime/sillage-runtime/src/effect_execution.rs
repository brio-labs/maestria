use crate::config::EffectExecutionContext;
use crate::effect_execution_dispatch::PreparedEffect;
use crate::effect_result::EffectFailure;
use crate::proposal_persistence::risk_class_to_approval_risk_level;
use sillage_domain::{
    DomainInput, LogicalTick, RequestApprovalRequest, SearchKnowledgeCompleted,
    SearchKnowledgeRequest, SillageEffect, UpdateGraphRequest,
};
use sillage_ports::{ApprovalRecord, ApprovalStatus};
use std::time::Duration;
use tokio::sync::mpsc;

impl EffectExecutionContext {
    /// Retry loop with timeout watchdog. Non-idempotent harness effects never
    /// replay automatically; their journal entry pauses or fails instead.
    pub(crate) async fn execute_with_retries(
        self,
        effect: SillageEffect,
    ) -> Result<(), EffectFailure> {
        let non_idempotent = matches!(
            &effect,
            SillageEffect::QueryHarness(_) | SillageEffect::QueryHarnessProposal(_)
        );
        let watchdog = self.default_effect_timeout + Duration::from_secs(5);
        let result = tokio::time::timeout(watchdog, async {
            let mut attempts: u32 = 0;
            loop {
                let is_busy = match self
                    .clone()
                    .execute_effect(effect.clone(), Some(self.default_effect_timeout))
                    .await
                {
                    Ok(()) => return Ok(()),
                    Err(error) => {
                        // Denied and degraded effects are expected,
                        // governed outcomes already reported by the
                        // admission and handler paths; logging them as
                        // execution errors at ERROR level would flood the
                        // log for every chunk of a large batch.
                        if matches!(error, EffectFailure::Denied(_) | EffectFailure::Degraded(_)) {
                            tracing::warn!(%error, "effect execution did not complete");
                        } else {
                            tracing::error!(%error, "effect execution did not complete");
                        }
                        let is_busy = match &error {
                            EffectFailure::Failed(message) => {
                                sillage_sqlite_support::is_database_busy(message)
                            }
                            EffectFailure::ApprovalLookup(port_error) => {
                                sillage_sqlite_support::is_database_busy(&port_error.to_string())
                            }
                            _ => false,
                        };
                        if is_busy {
                            if attempts >= sillage_sqlite_support::RETRY_ATTEMPTS {
                                return Err(error);
                            }
                        } else if !error.retryable()
                            || non_idempotent
                            || attempts >= self.max_retries
                        {
                            return Err(error);
                        }
                        is_busy
                    }
                };
                attempts += 1;
                tracing::warn!("Retrying effect execution (attempt {})", attempts);
                if is_busy {
                    tokio::time::sleep(sillage_sqlite_support::RETRY_DELAY).await;
                } else {
                    tokio::time::sleep(Duration::from_millis(500 * (1 << attempts))).await;
                }
            }
        })
        .await;

        match result {
            Ok(inner) => inner,
            Err(_) => {
                tracing::error!("Watchdog: effect execution timed out after {:?}", watchdog);
                if non_idempotent {
                    // No replay path exists for a harness effect: its journal
                    // entry must pause or fail, never vanish silently. Keep
                    // these timeouts fatal.
                    Err(EffectFailure::Failed("effect watchdog timeout".to_string()))
                } else {
                    // Idempotent effects (e.g. full-text indexing) are
                    // replayed by the watcher/recovery paths, so a timeout is
                    // a throughput degradation, not a corrupting failure;
                    // failing the whole runtime would kill the daemon on a
                    // saturated startup backlog.
                    Err(EffectFailure::Degraded(
                        "effect watchdog timeout".to_string(),
                    ))
                }
            }
        }
    }
    /// Execute the coalesced full-text work under the ordinary retry and
    /// watchdog policy for idempotent indexing effects.
    pub(crate) async fn execute_index_full_text_batch_with_retries(
        self,
        requests: Vec<sillage_domain::IndexChunkRequest>,
    ) -> Result<(), EffectFailure> {
        let watchdog = self.default_effect_timeout + Duration::from_secs(5);
        let result = tokio::time::timeout(watchdog, async {
            let mut attempts = 0;
            loop {
                for request in &requests {
                    let prepared = self
                        .prepare_effect(SillageEffect::IndexFullText(request.clone()))
                        .await?;
                    match prepared {
                        PreparedEffect::Dispatch { effect, .. }
                            if matches!(&*effect, SillageEffect::IndexFullText(_)) => {}
                        _ => {
                            return Err(EffectFailure::Failed(
                                "full-text batch admission prepared a non-index effect".to_string(),
                            ));
                        }
                    }
                }
                match self.handle_index_full_text_batch(requests.clone()).await {
                    Ok(()) => return Ok(()),
                    Err(error) if !error.retryable() || attempts >= self.max_retries => {
                        return Err(error);
                    }
                    Err(error) => {
                        attempts += 1;
                        tracing::warn!(%error, "retrying full-text batch execution");
                        tokio::time::sleep(Duration::from_millis(500 * (1 << attempts))).await;
                    }
                }
            }
        })
        .await;
        match result {
            Ok(inner) => inner,
            Err(_) => {
                tracing::error!("Watchdog: full-text batch timed out after {:?}", watchdog);
                Err(EffectFailure::Degraded(
                    "full-text batch watchdog timeout".to_string(),
                ))
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
            tracing::warn!(error = %e, context, "failed to send domain input (backpressure)");
            match e {
                mpsc::error::TrySendError::Full(_) => crate::FeedbackError::CapacityFull,
                mpsc::error::TrySendError::Closed(_) => crate::FeedbackError::RuntimeShutdown,
            }
        })
    }

    /// Ordered send into the domain input channel that awaits capacity.
    ///
    /// The artifact pipeline emits correlated inputs (ParserCompleted →
    /// RecordEvidence → StartFullTextIndex) that must reach the domain in
    /// order. Under parallel effect load `try_send` overflows the bounded
    /// channel and failing the effect would retry-storm; awaiting capacity
    /// applies backpressure while preserving order.
    pub(crate) async fn send_input_blocking(
        input_tx: &mpsc::Sender<DomainInput>,
        input: DomainInput,
        context: &'static str,
    ) -> Result<(), crate::FeedbackError> {
        match input_tx.try_send(input) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(pending)) => {
                input_tx.send(pending).await.map_err(|error| {
                    tracing::error!(%error, context, "failed to send domain input (shutdown)");
                    crate::FeedbackError::RuntimeShutdown
                })
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(crate::FeedbackError::RuntimeShutdown),
        }
    }

    // ── lightweight handlers ──────────────────────────────────────────

    pub(crate) async fn handle_search_knowledge(&self, request: SearchKnowledgeRequest) -> bool {
        let Some(executor) = &self.adapters.search_executor else {
            tracing::error!("search knowledge effect has no configured executor");
            return false;
        };
        let Some(plan) = self.confine_search_knowledge_plan(request.plan) else {
            return false;
        };
        self.execute_and_deliver_search(executor.as_ref(), plan, request.task_id)
            .await
    }

    fn confine_search_knowledge_plan(
        &self,
        plan: sillage_domain::SearchPlan,
    ) -> Option<sillage_domain::SearchPlan> {
        match plan.confine_to_scope(self.scope_id) {
            Ok(confined) => Some(confined),
            Err(error) => {
                tracing::error!(%error, "search knowledge scope confinement rejected");
                None
            }
        }
    }

    async fn execute_and_deliver_search(
        &self,
        executor: &dyn sillage_ports::SearchKnowledgeExecutor,
        plan: sillage_domain::SearchPlan,
        task_id: Option<sillage_domain::TaskId>,
    ) -> bool {
        let outcome = match executor.search(plan.clone()).await {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::error!(%error, "knowledge search failed");
                return false;
            }
        };
        if let Err(error) = outcome.verify_compatibility(&plan) {
            tracing::error!(%error, "search outcome is incompatible with request plan");
            return false;
        }
        Self::send_input(
            &self.input_tx,
            DomainInput::SearchKnowledgeCompleted(SearchKnowledgeCompleted {
                task_id,
                plan: Box::new(plan),
                outcome,
            }),
            "search knowledge completion",
        )
        .is_ok()
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

        let effect = SillageEffect::RequestApproval(RequestApprovalRequest {
            task_id: request.task_id,
        });
        let risk = self.governance.classifier.classify(&effect, &self.scope);
        let risk_level = risk_class_to_approval_risk_level(risk);

        let tick = {
            let state = self.state.read().await;
            match state.event_log.last() {
                Some(e) => LogicalTick::new(e.id.value()),
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
}
