use crate::config::EffectExecutionContext;
use crate::effect_execution_dispatch::PreparedEffect;
use crate::effect_result::EffectFailure;
use maestria_domain::{
    CorpusScope, DiagnosticEvent, DomainInput, LogicalTick, MaestriaEffect,
    QueryHarnessProposalRequest, RequestApprovalRequest, SearchKnowledgeCompleted,
    SearchKnowledgeRequest, UpdateGraphRequest,
};
use maestria_governance::{RiskClass, ScopeGuard};
use maestria_ports::{
    ApprovalRecord, ApprovalRiskLevel, ApprovalStatus, EffectJournalIntent, EffectJournalStatus,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingHarnessContinuation {
    proposal: maestria_domain::ModelAgentProposalRequest,
    journal_generation: u64,
    correlation_id: u64,
}

fn pending_capability(token: &PendingHarnessContinuation) -> Result<String, EffectFailure> {
    serde_json::to_string(token)
        .map(|json| format!("model_agent_pending:{json}"))
        .map_err(|error| EffectFailure::Failed(format!("encode pending proposal: {error}")))
}

pub(crate) fn decode_pending_continuation(
    record: &ApprovalRecord,
) -> Option<maestria_domain::ModelAgentProposalRequest> {
    let token = record
        .capability
        .strip_prefix("model_agent_pending:")
        .and_then(|json| serde_json::from_str::<PendingHarnessContinuation>(json).ok())?;
    let maestria_domain::ModelAgentProposalExecution::ApprovalContinuation {
        approval_id: _,
        journal_generation,
    } = token.proposal.execution
    else {
        return None;
    };
    if token.journal_generation != journal_generation
        || token.correlation_id != token.proposal.correlation_id
    {
        return None;
    }
    Some(token.proposal)
}

pub(crate) async fn persist_pending_harness(
    context: &EffectExecutionContext,
    request: &QueryHarnessProposalRequest,
) -> Result<(), EffectFailure> {
    let proposal = &request.proposal;
    if !matches!(
        &proposal.execution,
        maestria_domain::ModelAgentProposalExecution::Fresh
    ) {
        return Err(EffectFailure::Failed(
            "only a fresh proposal can create an approval continuation".to_string(),
        ));
    }
    let entry = context
        .adapters
        .effect_journal
        .record_intent(EffectJournalIntent {
            run_id: proposal.run_id,
            task_id: proposal.task_id,
            capability: proposal.capability.clone(),
            command: proposal.command.clone(),
            scope_id: context.scope_id,
            requested_generation: None,
        })
        .map_err(|error| {
            EffectFailure::Failed(format!("record pending harness intent: {error}"))
        })?;
    let approval_id = context
        .adapters
        .id_allocator
        .allocate_approval_id()
        .map_err(|error| EffectFailure::Failed(format!("allocate harness approval id: {error}")))?;
    let mut continuation = proposal.clone();
    continuation.execution = maestria_domain::ModelAgentProposalExecution::ApprovalContinuation {
        approval_id,
        journal_generation: entry.generation,
    };
    let token = PendingHarnessContinuation {
        proposal: continuation,
        journal_generation: entry.generation,
        correlation_id: proposal.correlation_id,
    };
    let capability = pending_capability(&token)?;
    let tick = {
        let state = context.state.read().await;
        state
            .event_log
            .last()
            .map_or(0, |event| event.sequence.value())
    };
    let scope_guard = ScopeGuard::new(context.scope.clone());
    let risk = context.governance.classifier.classify(
        &MaestriaEffect::QueryHarnessProposal(request.clone()),
        &scope_guard,
    );
    let record = ApprovalRecord {
        id: approval_id,
        task_id: proposal.task_id,
        effect_kind: "model_agent_harness".to_string(),
        risk_level: risk_class_to_approval_risk_level(risk),
        capability,
        scope_id: context.scope_id,
        tick: LogicalTick::new(tick),
        status: ApprovalStatus::Pending,
    };
    context
        .adapters
        .approval_repo
        .save(&record)
        .map_err(|error| EffectFailure::Failed(format!("persist harness approval: {error}")))?;
    tracing::info!(approval_id = %approval_id, correlation_id = proposal.correlation_id, "harness proposal pending approval");
    Ok(())
}

pub(crate) fn record_denied_harness(
    context: &EffectExecutionContext,
    request: &maestria_domain::QueryHarnessRequest,
) -> Result<(), EffectFailure> {
    let entry = context
        .adapters
        .effect_journal
        .record_intent(EffectJournalIntent {
            run_id: request.run_id,
            task_id: request.task_id,
            capability: request.capability.clone(),
            command: request.command.clone(),
            scope_id: request.scope_id,
            requested_generation: request.generation,
        })
        .map_err(|error| EffectFailure::Failed(format!("record denied harness intent: {error}")))?;
    context
        .adapters
        .effect_journal
        .record_started(request.run_id, entry.generation)
        .and_then(|_| {
            context.adapters.effect_journal.record_terminal(
                request.run_id,
                entry.generation,
                EffectJournalStatus::Failed,
            )
        })
        .map_err(|error| EffectFailure::Failed(format!("record denied harness terminal: {error}")))
}

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

fn risk_class_to_approval_risk_level(risk: RiskClass) -> ApprovalRiskLevel {
    match risk {
        RiskClass::Low => ApprovalRiskLevel::Low,
        RiskClass::Medium => ApprovalRiskLevel::Medium,
        RiskClass::High => ApprovalRiskLevel::High,
        RiskClass::Critical => ApprovalRiskLevel::Critical,
    }
}
