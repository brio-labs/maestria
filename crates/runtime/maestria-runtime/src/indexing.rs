use crate::config::EffectExecutionContext;
use maestria_domain::{DomainInput, FullTextIndexCompleted, IndexFullTextRequest};
use maestria_governance::scan_secrets;
use maestria_ports::{IndexedCard, IndexedChunk};

impl EffectExecutionContext {
    /// Index a chunk in the full-text search index.
    /// On the first chunk (order 0), also indexes all cards belonging
    /// to the artifact. Sends FullTextIndexCompleted back to the domain
    /// loop after the chunk is indexed.
    pub(crate) async fn handle_index_full_text(&self, request: IndexFullTextRequest) -> bool {
        let adapters = &self.adapters;
        let state = &self.state;

        let (chunk, artifact_security) = {
            let state = state.read().await;
            let Some(chunk) = state.chunks.get(&request.chunk_id).cloned() else {
                tracing::warn!(chunk_id = %request.chunk_id, "chunk missing for full-text index");
                return true;
            };
            let Some(artifact) = state.artifacts.get(&request.artifact_id) else {
                tracing::warn!(
                    artifact_id = %request.artifact_id,
                    "artifact missing for full-text index"
                );
                return false;
            };
            if chunk.artifact_id != request.artifact_id {
                tracing::warn!(
                    chunk_id = %request.chunk_id,
                    artifact_id = %request.artifact_id,
                    "chunk belongs to a different artifact"
                );
                return false;
            }
            (chunk, artifact.security.clone())
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
        if chunk.order == 0 {
            let artifact_cards: Vec<IndexedCard> = {
                let state = state.read().await;
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
                    return false;
                }
            }
            if !artifact_cards.is_empty()
                && let Err(error) = adapters.search_index.index_cards(artifact_cards)
            {
                tracing::error!(artifact_id = %request.artifact_id, %error, "failed to index cards");
                return false;
            }
        }
        if let Err(error) = adapters.search_index.index_chunks(vec![IndexedChunk {
            artifact_id: request.artifact_id,
            chunk_id: request.chunk_id,
            text: chunk.text,
        }]) {
            tracing::error!(chunk_id = %request.chunk_id, %error, "failed to index chunk");
            return false;
        }
        if Self::send_input(
            &self.input_tx,
            DomainInput::FullTextIndexCompleted(FullTextIndexCompleted {
                artifact_id: request.artifact_id,
                chunk_id: request.chunk_id,
            }),
            "full-text index completion",
        )
        .is_err()
        {
            return false;
        }
        true
    }
}
