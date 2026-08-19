use crate::config::EffectExecutionContext;
use maestria_domain::{
    Artifact, Chunk, DomainInput, EvidenceKind, FullTextIndexCompleted, IndexChunkRequest,
    evidence_id_for,
};
use maestria_governance::scan_secrets;
use maestria_ports::{IndexedCard, IndexedChunk, IndexedLexicalCard, IndexedLexicalChunk};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

impl EffectExecutionContext {
    /// Index one artifact's pending chunks in the full-text search index.
    ///
    /// The domain emits one `IndexFullText` effect per chunk, but the search
    /// index commits are the dominant per-artifact ingestion cost (segment
    /// flush and fsync per commit). The first effect for an artifact takes a
    /// per-artifact lock and indexes every still-pending chunk of the
    /// artifact in one atomic commit, then completes each indexed chunk; the
    /// sibling effects of the same artifact then observe their chunks
    /// completed and no-op. A per-chunk effect for a chunk that is no longer
    /// pending (already covered by an earlier batch, or re-driven after a
    /// crash) is an idempotent no-op.
    pub(crate) async fn handle_index_full_text(&self, request: IndexChunkRequest) -> bool {
        // Serialize same-artifact effects so exactly one of them runs the
        // batch; the others observe the completed chunks and no-op.
        let artifact_lock = {
            let mut locks = match self.full_text_locks.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            locks
                .entry(request.artifact_id)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _artifact_guard = artifact_lock.lock().await;

        let (artifact, pending) = {
            let state = self.state.read().await;
            let Some(artifact) = state.artifacts.get(&request.artifact_id).cloned() else {
                tracing::warn!(
                    artifact_id = %request.artifact_id,
                    "artifact missing for full-text index"
                );
                return false;
            };
            if !state.pending_full_text.contains(&request.chunk_id) {
                // Already indexed by an earlier batch of this artifact.
                return true;
            }
            let pending = state
                .chunks
                .values()
                .filter(|chunk| {
                    chunk.artifact_id == request.artifact_id
                        && state.pending_full_text.contains(&chunk.id)
                })
                .cloned()
                .collect::<Vec<_>>();
            (artifact, pending)
        };
        if pending.is_empty() {
            return true;
        }

        if !artifact.security.retrieval_allowed() {
            tracing::warn!(
                artifact_id = %request.artifact_id,
                "refusing full-text indexing for denied artifact"
            );
            return self.quarantine_and_complete(&request, &pending).await;
        }
        // A secret-bearing chunk quarantines the whole artifact before any
        // of its chunks are written.
        for chunk in &pending {
            let chunk_scan = scan_secrets(&chunk.text);
            if !chunk_scan.is_clean() {
                tracing::warn!(
                    chunk_id = %chunk.id,
                    findings = chunk_scan.findings.len(),
                    "refusing full-text indexing for secret-bearing chunk"
                );
                return self.quarantine_and_complete(&request, &pending).await;
            }
        }
        // Cards belong to the artifact, not to individual chunks; they are
        // registered once per artifact.
        let cards = match self.materialize_artifact_cards(&request).await {
            Some(cards) => cards,
            None => return false,
        };
        let (indexed_chunks, lexical_chunks, lexical_cards) =
            self.index_views_for_artifact(&artifact, &pending).await;
        if let Err(error) = self.adapters.search_index.index_artifact_chunks(
            indexed_chunks,
            cards,
            lexical_chunks,
            lexical_cards,
        ) {
            tracing::error!(
                artifact_id = %request.artifact_id,
                chunks = pending.len(),
                %error,
                "failed to index artifact chunks"
            );
            return false;
        }
        for chunk in &pending {
            if let Err(error) = Self::deliver_full_text_completion(
                &self.input_tx,
                FullTextIndexCompleted {
                    artifact_id: request.artifact_id,
                    chunk_id: chunk.id,
                },
            ) {
                tracing::error!(%error, "failed to deliver full-text index completion");
                return false;
            }
        }
        true
    }

    /// The indexed and lexical views for a whole artifact's pending chunks:
    /// one `IndexedChunk`/`IndexedLexicalChunk` per chunk and one
    /// `IndexedLexicalCard` per card, deterministically ordered by chunk id.
    async fn index_views_for_artifact(
        &self,
        artifact: &Artifact,
        pending: &[Chunk],
    ) -> (
        Vec<IndexedChunk>,
        Vec<IndexedLexicalChunk>,
        Vec<IndexedLexicalCard>,
    ) {
        let source_paths = {
            let state = self.state.read().await;
            pending
                .iter()
                .map(|chunk| {
                    let source_path = state
                        .evidences
                        .get(&evidence_id_for(artifact.id, chunk.order))
                        .and_then(|evidence| match &evidence.kind {
                            EvidenceKind::FileSpan { path, .. } => Some(path.clone()),
                            _ => None,
                        });
                    (chunk.id, source_path)
                })
                .collect::<BTreeMap<_, _>>()
        };
        let supports_lexical = self.adapters.search_index.supports_lexical_metadata();
        let mut indexed_chunks = Vec::with_capacity(pending.len());
        let mut lexical_chunks = Vec::with_capacity(pending.len());
        let mut lexical_cards = Vec::new();
        for chunk in pending {
            let source_path = source_paths.get(&chunk.id).cloned().flatten();
            indexed_chunks.push(IndexedChunk {
                artifact_id: artifact.id,
                chunk_id: chunk.id,
                text: chunk.text.clone(),
            });
            if supports_lexical {
                lexical_chunks.push(IndexedLexicalChunk {
                    artifact_id: artifact.id,
                    chunk_id: chunk.id,
                    text: chunk.text.clone(),
                    path: source_path.clone(),
                    filename: Self::file_name_of(source_path.as_deref()),
                    symbol: None,
                });
            }
        }
        if supports_lexical {
            let cards = {
                let state = self.state.read().await;
                state
                    .cards
                    .values()
                    .filter(|card| card.artifact_id == artifact.id)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            for card in cards {
                lexical_cards.push(IndexedLexicalCard {
                    artifact_id: artifact.id,
                    card_id: card.id,
                    title: card.title,
                    body: card.body,
                    path: None,
                    filename: None,
                    symbol: None,
                });
            }
        }
        (indexed_chunks, lexical_chunks, lexical_cards)
    }

    fn file_name_of(path: Option<&str>) -> Option<String> {
        path.and_then(|path| {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
    }

    /// Terminalize a refused artifact without failing the effect.
    ///
    /// The artifact's chunks are deliberately never written to the search
    /// index; the artifact is marked `Quarantined` (idempotent — the domain
    /// emits no duplicate events once the status is recorded) and every
    /// pending chunk's indexing pipeline completes so the artifact reaches a
    /// terminal state and the batch continues. A refusal is a per-artifact
    /// privacy outcome, not a runtime failure.
    async fn quarantine_and_complete(
        &self,
        request: &IndexChunkRequest,
        pending: &[Chunk],
    ) -> bool {
        if let Some(artifact) = self.state.read().await.artifacts.get(&request.artifact_id) {
            let Some(hash) = artifact.content_hash.clone() else {
                return false;
            };
            if Self::send_input_blocking(
                &self.input_tx,
                DomainInput::ParserCompleted(maestria_domain::ParserResult {
                    artifact_id: request.artifact_id,
                    artifact_version_id: maestria_domain::ArtifactVersionId::new(
                        request.artifact_id.value(),
                    ),
                    content_hash: hash,
                    status: maestria_domain::ParseStatus::Quarantined,
                    tree_root_id: None,
                    tree_nodes: Vec::new(),
                    chunks: Vec::new(),
                    cards: Vec::new(),
                }),
                "quarantine artifact",
            )
            .await
            .is_err()
            {
                return false;
            }
        } else {
            return false;
        }
        for chunk in pending {
            let completion = FullTextIndexCompleted {
                artifact_id: request.artifact_id,
                chunk_id: chunk.id,
            };
            if Self::deliver_full_text_completion(&self.input_tx, completion).is_err() {
                return false;
            }
        }
        true
    }

    /// Deliver a committed full-text completion to the domain input loop.
    /// On `CapacityFull` a detached task awaits channel capacity and the
    /// effect succeeds: retrying would re-run the expensive
    /// `index_artifact_chunk` commit under a permit (the #421 retry storm).
    /// A closed channel still fails — the runtime is shutting down anyway.
    fn deliver_full_text_completion(
        input_tx: &mpsc::Sender<DomainInput>,
        completion: FullTextIndexCompleted,
    ) -> Result<(), crate::FeedbackError> {
        match Self::send_input(
            input_tx,
            DomainInput::FullTextIndexCompleted(completion.clone()),
            "full-text index completion",
        ) {
            Ok(()) => Ok(()),
            Err(crate::FeedbackError::CapacityFull) => {
                let input_tx = input_tx.clone();
                tokio::spawn(async move {
                    if let Err(error) = input_tx
                        .send(DomainInput::FullTextIndexCompleted(completion))
                        .await
                    {
                        tracing::warn!(
                            %error,
                            "full-text index completion dropped; runtime input channel closed"
                        );
                    }
                });
                Ok(())
            }
            Err(error @ crate::FeedbackError::RuntimeShutdown) => Err(error),
        }
    }

    /// Materialize the artifact's cards for full-text indexing, refusing the
    /// whole artifact when any card carries secret-like content (the runtime
    /// shuts down on secret-bearing indexing).
    async fn materialize_artifact_cards(
        &self,
        request: &IndexChunkRequest,
    ) -> Option<Vec<IndexedCard>> {
        let artifact_cards: Vec<IndexedCard> = {
            let state = self.state.read().await;
            state
                .cards
                .values()
                .filter(|c| c.artifact_id == request.artifact_id)
                .map(|c| IndexedCard {
                    artifact_id: c.artifact_id,
                    card_id: c.id,
                    title: c.title.clone(),
                    body: c.body.clone(),
                })
                .collect()
        };
        for card in &artifact_cards {
            let title_scan = scan_secrets(&card.title);
            let body_scan = scan_secrets(&card.body);
            if !title_scan.is_clean() || !body_scan.is_clean() {
                tracing::warn!(
                    card_id = %card.card_id,
                    "refusing full-text indexing for secret-bearing card"
                );
                return None;
            }
        }
        Some(artifact_cards)
    }
}
