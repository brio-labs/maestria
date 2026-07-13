use crate::config::EffectExecutionContext;
use maestria_domain::{
    ApprovalDecision, DiagnosticEvent, DomainInput, FetchWebRequest, IndexVectorRequest,
    MaestriaEffect, RequestApprovalRequest, UpdateGraphRequest,
};
use maestria_governance::{ApprovalRequest, PolicyDecision, ScopeGuard};
use maestria_ports::VectorEmbedding;
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

    /// Retry loop with timeout watchdog. Persistence effects bypass
    /// the semaphore in the run loop; non-persistence effects always
    /// retry on failure up to `max_retries`.
    pub(crate) async fn execute_with_retries(self, effect: MaestriaEffect) -> bool {
        let watchdog = self.default_effect_timeout + Duration::from_secs(5);
        let result = tokio::time::timeout(watchdog, async {
            let mut attempts = 0;
            loop {
                let success = self
                    .clone()
                    .execute_effect(effect.clone(), Some(self.default_effect_timeout))
                    .await;

                if success || attempts >= self.max_retries {
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
    pub(crate) async fn send_input(
        input_tx: &mpsc::Sender<DomainInput>,
        input: DomainInput,
        context: &'static str,
    ) {
        if let Err(error) = input_tx.send(input).await {
            tracing::error!(%error, context, "failed to send domain input");
        }
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
        tracing::info!(task_id = %request.task_id, "approval request requires external decision");
        Self::send_input(
            &self.input_tx,
            DomainInput::ApprovalResolved(ApprovalDecision {
                task_id: request.task_id,
                approved: false,
            }),
            "approval decision",
        )
        .await;
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
        let chunk = {
            let state = self.state.read().await;
            state.chunks.get(&request.chunk_id).cloned()
        };
        let Some(chunk) = chunk else {
            tracing::warn!(chunk_id = %request.chunk_id, "chunk missing for vector index");
            return true;
        };
        let embedding = VectorEmbedding {
            chunk_id: request.chunk_id,
            vector: Vec::new(),
            provenance: maestria_ports::EmbeddingProvenance {
                content_hash: String::new(),
                model_version: String::new(),
            },
        };
        tracing::info!(
            chunk_id = %request.chunk_id,
            text_len = chunk.text.len(),
            "indexing chunk in vector store (no embedding provider configured; storing empty vector)"
        );
        if let Err(error) = self.adapters.vector_index.index_embeddings(vec![embedding]) {
            tracing::error!(chunk_id = %request.chunk_id, %error, "failed to index vector");
            return false;
        }
        true
    }
}
