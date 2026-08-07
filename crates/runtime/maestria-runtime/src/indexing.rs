use crate::config::EffectExecutionContext;
use maestria_domain::{
    Chunk, DomainInput, EvidenceKind, FullTextIndexCompleted, IndexFullTextRequest,
    SecurityMetadata, evidence_id_for,
};
use maestria_governance::scan_secrets;
use maestria_ports::{IndexedCard, IndexedChunk, IndexedLexicalCard, IndexedLexicalChunk};
use std::path::Path;
use tokio::sync::mpsc;

impl EffectExecutionContext {
    /// Index a chunk in the full-text search index.
    /// On the first chunk (order 0), also indexes all cards belonging
    /// to the artifact. Sends FullTextIndexCompleted back to the domain
    /// loop after the chunk is indexed.
    ///
    /// The chunk, its cards, and their lexical metadata are written through
    /// the port's single-commit `index_artifact_chunk` batch: the search
    /// index commits are the dominant per-artifact cost (segment flush and
    /// fsync per commit), so one atomic update per artifact chunk replaces
    /// four separate commits. The delete-then-add pattern keeps re-drives
    /// idempotent, and the update becomes visible atomically.
    pub(crate) async fn handle_index_full_text(&self, request: IndexFullTextRequest) -> bool {
        let (chunk, artifact_security, source_path) =
            match self.extract_index_metadata(&request).await {
                Ok(meta) => meta,
                Err(early_return) => return early_return,
            };

        if !artifact_security.retrieval_allowed() {
            tracing::warn!(
                artifact_id = %request.artifact_id,
                "refusing full-text indexing for denied artifact"
            );
            return false;
        }
        let chunk_scan = scan_secrets(&chunk.text);
        if !chunk_scan.is_clean() {
            tracing::warn!(
                chunk_id = %request.chunk_id,
                findings = chunk_scan.findings.len(),
                "refusing full-text indexing for secret-bearing chunk"
            );
            return false;
        }
        // Cards belong to the artifact, not to individual chunks; index them
        // only on the first chunk so they are registered once per artifact.
        let cards = if chunk.order == 0 {
            match self.materialize_artifact_cards(&request).await {
                Some(cards) => cards,
                None => return false,
            }
        } else {
            Vec::new()
        };
        let (lexical_cards, lexical_chunk) =
            self.lexical_index_views(&request, &cards, &chunk, source_path);
        if let Err(error) = self.adapters.search_index.index_artifact_chunk(
            IndexedChunk {
                artifact_id: request.artifact_id,
                chunk_id: request.chunk_id,
                text: chunk.text,
            },
            cards,
            lexical_chunk,
            lexical_cards,
        ) {
            tracing::error!(
                artifact_id = %request.artifact_id,
                chunk_id = %request.chunk_id,
                %error,
                "failed to index artifact chunk"
            );
            return false;
        }
        if let Err(error) = Self::deliver_full_text_completion(
            &self.input_tx,
            FullTextIndexCompleted {
                artifact_id: request.artifact_id,
                chunk_id: request.chunk_id,
            },
        ) {
            tracing::error!(%error, "failed to deliver full-text index completion");
            return false;
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
        request: &IndexFullTextRequest,
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

    /// Build the lexical metadata views for the artifact's cards and the
    /// current chunk, empty when the search index does not support lexical
    /// metadata.
    fn lexical_index_views(
        &self,
        request: &IndexFullTextRequest,
        cards: &[IndexedCard],
        chunk: &Chunk,
        source_path: Option<String>,
    ) -> (Vec<IndexedLexicalCard>, Option<IndexedLexicalChunk>) {
        if !self.adapters.search_index.supports_lexical_metadata() {
            return (Vec::new(), None);
        }
        let filename = source_path
            .as_deref()
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .map(str::to_string);
        let lexical_cards = cards
            .iter()
            .map(|card| IndexedLexicalCard {
                artifact_id: card.artifact_id,
                card_id: card.card_id,
                title: card.title.clone(),
                body: card.body.clone(),
                path: source_path.clone(),
                filename: filename.clone(),
                symbol: None,
            })
            .collect();
        let lexical_chunk = Some(IndexedLexicalChunk {
            artifact_id: request.artifact_id,
            chunk_id: request.chunk_id,
            text: chunk.text.clone(),
            path: source_path,
            filename,
            symbol: None,
        });
        (lexical_cards, lexical_chunk)
    }

    async fn extract_index_metadata(
        &self,
        request: &IndexFullTextRequest,
    ) -> Result<(Chunk, SecurityMetadata, Option<String>), bool> {
        let state = self.state.read().await;
        let Some(chunk) = state.chunks.get(&request.chunk_id).cloned() else {
            tracing::error!(
                chunk_id = %request.chunk_id,
                "chunk missing for full-text index; effect cannot complete"
            );
            return Err(false);
        };
        let Some(artifact) = state.artifacts.get(&request.artifact_id) else {
            tracing::warn!(
                artifact_id = %request.artifact_id,
                "artifact missing for full-text index"
            );
            return Err(false);
        };
        if chunk.artifact_id != request.artifact_id {
            tracing::warn!(
                chunk_id = %request.chunk_id,
                artifact_id = %request.artifact_id,
                "chunk belongs to a different artifact"
            );
            return Err(false);
        }
        let source_path = state
            .evidences
            .get(&evidence_id_for(request.artifact_id, chunk.order))
            .and_then(|evidence| match &evidence.kind {
                EvidenceKind::FileSpan { path, .. } => Some(path.clone()),
                _ => None,
            });
        Ok((chunk, artifact.security.clone(), source_path))
    }
}
