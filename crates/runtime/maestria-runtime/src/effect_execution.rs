use crate::config::EffectExecutionContext;
use maestria_domain::{
    CorpusScope, DiagnosticEvent, DomainInput, IndexVectorRequest, LogicalTick, MaestriaEffect,
    RequestApprovalRequest, SearchKnowledgeCompleted, SearchKnowledgeRequest, UpdateGraphRequest,
};
use maestria_governance::{ApprovalRequest, PolicyDecision, RiskClass, ScopeGuard, scan_secrets};
use maestria_ports::{ApprovalRecord, ApprovalRiskLevel, ApprovalStatus, VectorEmbedding};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EffectFailure {
    Denied(String),
    RequiresApproval(String),
    Failed(String),
    Degraded(String),
}

impl EffectFailure {
    fn retryable(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

impl std::fmt::Display for EffectFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied(reason) => write!(formatter, "effect denied: {reason}"),
            Self::RequiresApproval(reason) => {
                write!(formatter, "effect requires approval: {reason}")
            }
            Self::Failed(reason) => write!(formatter, "effect failed: {reason}"),
            Self::Degraded(reason) => write!(formatter, "effect degraded: {reason}"),
        }
    }
}

fn handler_result(success: bool, effect_name: &'static str) -> Result<(), EffectFailure> {
    if success {
        Ok(())
    } else {
        Err(EffectFailure::Failed(effect_name.to_string()))
    }
}

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
    pub(crate) async fn execute_with_retries(self, effect: MaestriaEffect) -> bool {
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
                    Ok(()) => return true,
                    Err(error) => {
                        tracing::error!(%error, "effect execution did not complete");
                        if !error.retryable() || non_idempotent || attempts >= self.max_retries {
                            return false;
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

    async fn handle_index_vector(&self, request: IndexVectorRequest) -> Result<(), EffectFailure> {
        let Some(provider) = &self.adapters.embedding_provider else {
            tracing::debug!(chunk_id = %request.chunk_id, "vector indexing disabled");
            return self
                .degraded_after_invalidation(
                    request.chunk_id,
                    "embedding provider is not configured",
                )
                .await;
        };
        let Some(model) = self
            .embedding_model
            .clone()
            .filter(|model| !model.trim().is_empty())
        else {
            tracing::warn!(chunk_id = %request.chunk_id, "vector provider configured without model");
            return self
                .degraded_after_invalidation(request.chunk_id, "embedding model is not configured")
                .await;
        };
        let (chunk, content_hash) = self.load_vector_chunk(request.chunk_id).await?;
        let Some(identity) = provider.identity() else {
            tracing::warn!(chunk_id = %request.chunk_id, "embedding provider has no generation identity");
            return self
                .degraded_after_invalidation(
                    request.chunk_id,
                    "embedding provider has no generation identity",
                )
                .await;
        };
        let embedding_request = maestria_ports::EmbeddingRequest {
            text: chunk.text.clone(),
            model,
            kind: maestria_ports::EmbeddingInputKind::Document,
            identity: identity.clone(),
        };
        let provider = Arc::clone(provider);
        let response = match embed_blocking(provider, embedding_request).await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(chunk_id = %request.chunk_id, %error, "embedding provider failed; preserving fallback");
                return self
                    .degraded_after_invalidation(request.chunk_id, "embedding provider failed")
                    .await;
            }
        };
        if response.identity != identity {
            return self
                .degraded_after_invalidation(
                    request.chunk_id,
                    "embedding response identity did not match the active generation",
                )
                .await;
        }
        let embedding = VectorEmbedding {
            chunk_id: request.chunk_id,
            vector: response.vector,
            provenance: maestria_ports::EmbeddingProvenance {
                content_hash,
                identity: response.identity,
                provider_id: response.provider_id,
                model: response.model,
                model_version: response.model_version,
                disclosure: response.disclosure,
            },
        };
        self.index_vector_embedding(request.chunk_id, embedding)
            .await
    }

    async fn index_vector_embedding(
        &self,
        chunk_id: maestria_domain::ChunkId,
        embedding: VectorEmbedding,
    ) -> Result<(), EffectFailure> {
        let vector_index = Arc::clone(&self.adapters.vector_index);
        match tokio::task::spawn_blocking(move || vector_index.index_embeddings(vec![embedding]))
            .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                tracing::warn!(chunk_id = %chunk_id, %error, "vector projection failed; preserving fallback");
                self.degraded_after_invalidation(chunk_id, "vector projection failed")
                    .await
            }
            Err(error) => {
                tracing::warn!(chunk_id = %chunk_id, %error, "vector projection task failed; preserving fallback");
                self.degraded_after_invalidation(chunk_id, "vector projection task failed")
                    .await
            }
        }
    }

    async fn degraded_after_invalidation(
        &self,
        chunk_id: maestria_domain::ChunkId,
        reason: &'static str,
    ) -> Result<(), EffectFailure> {
        if self.invalidate_vector_projection(chunk_id).await {
            Err(EffectFailure::Degraded(reason.to_string()))
        } else {
            Err(EffectFailure::Failed(format!(
                "{reason}; stale vector projection could not be invalidated"
            )))
        }
    }

    async fn load_vector_chunk(
        &self,
        chunk_id: maestria_domain::ChunkId,
    ) -> Result<(maestria_domain::Chunk, String), EffectFailure> {
        let (chunk, content_hash, security_allowed) = {
            let state = self.state.read().await;
            let Some(chunk) = state.chunks.get(&chunk_id).cloned() else {
                return Err(EffectFailure::Failed(format!(
                    "chunk {chunk_id} is missing"
                )));
            };
            let (content_hash, security_allowed) = match state.artifacts.get(&chunk.artifact_id) {
                Some(artifact) => {
                    let content_hash = match artifact.content_hash.clone() {
                        Some(content_hash) => content_hash,
                        None => maestria_domain::content_hash(chunk.text.as_bytes()),
                    };
                    (content_hash, artifact.security.retrieval_allowed())
                }
                None => (maestria_domain::content_hash(chunk.text.as_bytes()), false),
            };
            (chunk, content_hash, security_allowed)
        };
        if !security_allowed {
            tracing::warn!(chunk_id = %chunk_id, "refusing vector indexing for denied artifact");
            return Err(EffectFailure::Failed(
                "artifact is not allowed for retrieval".to_string(),
            ));
        }
        let secret_scan = scan_secrets(&chunk.text);
        if !secret_scan.is_clean() {
            tracing::warn!(
                chunk_id = %chunk_id,
                findings = secret_scan.findings.len(),
                "refusing embedding for secret-bearing chunk"
            );
            return Err(EffectFailure::Failed(
                "chunk contains secret-like content".to_string(),
            ));
        }
        Ok((chunk, content_hash))
    }

    async fn invalidate_vector_projection(&self, chunk_id: maestria_domain::ChunkId) -> bool {
        let vector_index = Arc::clone(&self.adapters.vector_index);
        let result =
            tokio::task::spawn_blocking(move || vector_index.delete_chunks(&[chunk_id])).await;
        match result {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::warn!(chunk_id = %chunk_id, %error, "could not invalidate stale vector projection");
                false
            }
            Err(error) => {
                tracing::warn!(chunk_id = %chunk_id, %error, "vector invalidation task failed");
                false
            }
        }
    }
}

async fn embed_blocking(
    provider: Arc<dyn maestria_ports::EmbeddingProvider + Send + Sync>,
    request: maestria_ports::EmbeddingRequest,
) -> Result<maestria_ports::EmbeddingResponse, maestria_ports::PortError> {
    match tokio::task::spawn_blocking(move || provider.embed(request)).await {
        Ok(result) => result,
        Err(error) => Err(maestria_ports::PortError::Internal {
            message: format!("embedding provider task failed: {error}"),
        }),
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
