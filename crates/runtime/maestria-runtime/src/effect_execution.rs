use crate::config::EffectExecutionContext;
use maestria_domain::{
    DiagnosticEvent, DomainInput, FetchWebRequest, IndexVectorRequest, LogicalTick, MaestriaEffect,
    RequestApprovalRequest, UpdateGraphRequest,
};
use maestria_governance::{ApprovalRequest, PolicyDecision, RiskClass, ScopeGuard};
use maestria_ports::{ApprovalRecord, ApprovalRiskLevel, ApprovalStatus, VectorEmbedding};
use std::{sync::Arc, time::Duration};
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
    ) -> bool {
        // ── governance gate ──────────────────────────────────────────
        let scope = ScopeGuard::new(self.scope.clone());
        let risk = self.governance.classifier.classify(&effect, &scope);
        let decision = self.governance.approval_gate.decide(&ApprovalRequest {
            effect: &effect,
            profile: self.profile,
            scope: &scope,
        });

        let persistence_effect = matches!(&effect, MaestriaEffect::PersistEvent { .. });
        match decision.decision {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => {
                tracing::warn!(?risk, %reason, "effect denied");
                return !persistence_effect;
            }
            PolicyDecision::RequireApproval { reason } => {
                tracing::info!(?risk, %reason, "effect requires approval");
                return !persistence_effect;
            }
        }

        match effect {
            MaestriaEffect::PersistEvent { envelope } => self.handle_persist_event(envelope).await,
            MaestriaEffect::PersistState(request) => self.handle_persist_state(request).await,
            MaestriaEffect::ParseArtifact(request) => {
                self.handle_parse_artifact(request, persistence_barrier_timeout)
                    .await
            }
            MaestriaEffect::IndexFullText(request) => self.handle_index_full_text(request).await,
            MaestriaEffect::IndexVector(request) => self.handle_index_vector(request).await,
            MaestriaEffect::UpdateGraph(request) => self.handle_update_graph(request).await,
            MaestriaEffect::QueryHarness(request) => self.handle_query_harness(request).await,
            MaestriaEffect::FetchWeb(request) => self.handle_fetch_web(request).await,
            MaestriaEffect::RunValidation(request) => self.handle_run_validation(request).await,
            MaestriaEffect::RequestApproval(request) => self.handle_request_approval(request).await,
            MaestriaEffect::EmitDiagnostic(diagnostic) => {
                self.handle_emit_diagnostic(diagnostic).await
            }
        }
    }

    /// Retry loop with timeout watchdog. Non-idempotent harness effects never
    /// replay automatically; their journal entry pauses or fails instead.
    pub(crate) async fn execute_with_retries(self, effect: MaestriaEffect) -> bool {
        let non_idempotent = matches!(&effect, MaestriaEffect::QueryHarness(_));
        let watchdog = self.default_effect_timeout + Duration::from_secs(5);
        let result = tokio::time::timeout(watchdog, async {
            let mut attempts = 0;
            loop {
                let success = self
                    .clone()
                    .execute_effect(effect.clone(), Some(self.default_effect_timeout))
                    .await;

                if success || non_idempotent || attempts >= self.max_retries {
                    return success;
                }
                attempts += 1;
                tracing::warn!("Retrying effect execution (attempt {})", attempts);
                tokio::time::sleep(Duration::from_millis(500 * (1 << attempts))).await;
            }
        })
        .await;

        match result {
            Ok(success) => success,
            Err(_) => {
                tracing::error!(
                    "Watchdog: effect execution timed out after {:?}",
                    self.default_effect_timeout
                );
                false
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

    async fn handle_update_graph(&self, request: UpdateGraphRequest) -> bool {
        let relation = {
            let state = self.state.read().await;
            state.relations.get(&request.relation_id).cloned()
        };
        let Some(relation) = relation else {
            tracing::warn!(relation_id = %request.relation_id, "relation missing for graph update");
            return true;
        };
        if let Err(error) = self.adapters.graph_index.insert_relation(relation) {
            tracing::error!(relation_id = %request.relation_id, %error, "failed to insert relation into graph");
            return false;
        }
        true
    }

    async fn handle_fetch_web(&self, request: FetchWebRequest) -> bool {
        match self.adapters.web_fetcher.fetch(&request.url) {
            Ok(snapshot) => {
                tracing::debug!(
                    url = %request.url,
                    html_len = snapshot.html.len(),
                    "web fetch succeeded"
                );
                true
            }
            Err(error) => {
                tracing::error!(url = %request.url, %error, "web fetch failed");
                false
            }
        }
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

    async fn handle_index_vector(&self, request: IndexVectorRequest) -> bool {
        let Some(provider) = &self.adapters.embedding_provider else {
            tracing::debug!(chunk_id = %request.chunk_id, "vector indexing disabled");
            return true;
        };
        let Some(model) = self
            .embedding_model
            .clone()
            .filter(|model| !model.trim().is_empty())
        else {
            tracing::warn!(chunk_id = %request.chunk_id, "vector provider configured without model");
            return true;
        };
        let (chunk, content_hash) = {
            let state = self.state.read().await;
            let Some(chunk) = state.chunks.get(&request.chunk_id).cloned() else {
                return {
                    tracing::warn!(chunk_id = %request.chunk_id, "chunk missing for vector index");
                    true
                };
            };
            let content_hash = match state
                .artifacts
                .get(&chunk.artifact_id)
                .and_then(|artifact| artifact.content_hash.clone())
            {
                Some(content_hash) => content_hash,
                None => maestria_domain::content_hash(chunk.text.as_bytes()),
            };
            (chunk, content_hash)
        };
        let embedding_request = maestria_ports::EmbeddingRequest {
            text: chunk.text.clone(),
            model,
        };
        let provider = Arc::clone(provider);
        let response = match tokio::task::spawn_blocking(move || provider.embed(embedding_request))
            .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                tracing::warn!(chunk_id = %request.chunk_id, %error, "embedding provider failed; preserving fallback");
                self.invalidate_vector_projection(request.chunk_id).await;
                return true;
            }
            Err(error) => {
                tracing::warn!(chunk_id = %request.chunk_id, %error, "embedding provider task failed; preserving fallback");
                self.invalidate_vector_projection(request.chunk_id).await;
                return true;
            }
        };
        let embedding = VectorEmbedding {
            chunk_id: request.chunk_id,
            vector: response.vector,
            provenance: maestria_ports::EmbeddingProvenance {
                content_hash,
                provider_id: response.provider_id,
                model: response.model,
                model_version: response.model_version,
            },
        };
        let vector_index = Arc::clone(&self.adapters.vector_index);
        match tokio::task::spawn_blocking(move || vector_index.index_embeddings(vec![embedding]))
            .await
        {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::warn!(chunk_id = %request.chunk_id, %error, "vector projection failed; preserving fallback");
                self.invalidate_vector_projection(request.chunk_id).await;
                true
            }
            Err(error) => {
                tracing::warn!(chunk_id = %request.chunk_id, %error, "vector projection task failed; preserving fallback");
                self.invalidate_vector_projection(request.chunk_id).await;
                true
            }
        }
    }

    async fn invalidate_vector_projection(&self, chunk_id: maestria_domain::ChunkId) {
        let vector_index = Arc::clone(&self.adapters.vector_index);
        let result =
            tokio::task::spawn_blocking(move || vector_index.delete_chunks(&[chunk_id])).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(chunk_id = %chunk_id, %error, "could not invalidate stale vector projection");
            }
            Err(error) => {
                tracing::debug!(chunk_id = %chunk_id, %error, "vector invalidation task failed");
            }
        }
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
