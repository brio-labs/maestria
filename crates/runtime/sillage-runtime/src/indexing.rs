use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use sillage_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, DomainInput, EvidenceKind, FullTextIndexCompleted,
    IndexChunkRequest, evidence_id_for,
};
use sillage_governance::scan_secrets;
use sillage_ports::{IndexedCard, IndexedChunk, IndexedLexicalCard, IndexedLexicalChunk};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tokio::sync::mpsc;

struct FullTextArtifactUpdate {
    artifact_id: ArtifactId,
    pending: Vec<Chunk>,
    indexed_chunks: Vec<IndexedChunk>,
    cards: Vec<IndexedCard>,
    lexical_chunks: Vec<IndexedLexicalChunk>,
    lexical_cards: Vec<IndexedLexicalCard>,
}

impl EffectExecutionContext {
    /// Publish all currently admitted full-text effects as one ingestion-side
    /// batch. The serialized owner commits and reloads once before it delivers
    /// any successful completion, so `Indexed` always implies search visibility.
    pub(crate) async fn handle_index_full_text_batch(
        &self,
        requests: Vec<IndexChunkRequest>,
    ) -> Result<(), EffectFailure> {
        let _batch_guard = self.full_text_batch_lock.lock().await;
        let mut requested_chunks = BTreeMap::<ArtifactId, BTreeSet<ChunkId>>::new();
        for request in requests {
            requested_chunks
                .entry(request.artifact_id)
                .or_default()
                .insert(request.chunk_id);
        }

        let mut updates = Vec::new();
        for (artifact_id, request_chunk_ids) in requested_chunks {
            let Some((artifact, pending)) = self
                .resolve_pending_chunks_for_index(artifact_id, &request_chunk_ids)
                .await
            else {
                return Err(EffectFailure::Failed(format!(
                    "artifact {artifact_id} is missing for full-text indexing"
                )));
            };
            if pending.is_empty() {
                continue;
            }
            if !self
                .validate_indexing_safety(artifact_id, &artifact, &pending)
                .await?
            {
                continue;
            }
            let cards = self
                .materialize_artifact_cards(artifact_id)
                .await
                .ok_or_else(|| {
                    EffectFailure::Failed(format!(
                        "refusing full-text indexing for secret-bearing card in artifact {artifact_id}"
                    ))
                })?;
            let (indexed_chunks, lexical_chunks, lexical_cards) =
                self.index_views_for_artifact(&artifact, &pending).await;
            updates.push(FullTextArtifactUpdate {
                artifact_id,
                pending,
                indexed_chunks,
                cards,
                lexical_chunks,
                lexical_cards,
            });
        }

        for update in &mut updates {
            self.adapters
                .search_index
                .index_artifact_chunks(
                    std::mem::take(&mut update.indexed_chunks),
                    std::mem::take(&mut update.cards),
                    std::mem::take(&mut update.lexical_chunks),
                    std::mem::take(&mut update.lexical_cards),
                )
                .map_err(|error| {
                    EffectFailure::Failed(format!("failed to buffer full-text artifact: {error}"))
                })?;
        }
        if !updates.is_empty() {
            self.adapters
                .search_index
                .commit_and_reload()
                .map_err(|error| {
                    EffectFailure::Failed(format!(
                        "failed to commit and reload full-text batch: {error}"
                    ))
                })?;
        }
        for update in &updates {
            if !self.deliver_all_full_text_completions(update.artifact_id, &update.pending) {
                return Err(EffectFailure::Failed(
                    "failed to deliver full-text index completion".to_string(),
                ));
            }
        }
        Ok(())
    }

    async fn resolve_pending_chunks_for_index(
        &self,
        artifact_id: ArtifactId,
        request_chunk_ids: &BTreeSet<ChunkId>,
    ) -> Option<(Artifact, Vec<Chunk>)> {
        let state = self.state.read().await;
        let Some(artifact) = state.artifacts.get(&artifact_id).cloned() else {
            tracing::warn!(
                artifact_id = %artifact_id,
                "artifact missing for full-text index"
            );
            return None;
        };
        if !request_chunk_ids
            .iter()
            .any(|chunk_id| state.pending_full_text.contains(chunk_id))
        {
            return Some((artifact, Vec::new()));
        }
        let pending = state
            .chunks
            .values()
            .filter(|chunk| {
                chunk.artifact_id == artifact_id && state.pending_full_text.contains(&chunk.id)
            })
            .cloned()
            .collect::<Vec<_>>();
        Some((artifact, pending))
    }

    async fn validate_indexing_safety(
        &self,
        artifact_id: ArtifactId,
        artifact: &Artifact,
        pending: &[Chunk],
    ) -> Result<bool, EffectFailure> {
        if !artifact.security.retrieval_allowed() {
            tracing::warn!(
                artifact_id = %artifact_id,
                "refusing full-text indexing for denied artifact"
            );
            return self
                .quarantine_and_complete(artifact_id, pending)
                .await
                .then_some(false)
                .ok_or_else(|| {
                    EffectFailure::Failed(format!(
                        "failed to quarantine denied artifact {artifact_id}"
                    ))
                });
        }
        for chunk in pending {
            let chunk_scan = scan_secrets(&chunk.text);
            if !chunk_scan.is_clean() {
                tracing::warn!(
                    chunk_id = %chunk.id,
                    findings = chunk_scan.findings.len(),
                    "refusing full-text indexing for secret-bearing chunk"
                );
                return self
                    .quarantine_and_complete(artifact_id, pending)
                    .await
                    .then_some(false)
                    .ok_or_else(|| {
                        EffectFailure::Failed(format!(
                            "failed to quarantine secret-bearing artifact {artifact_id}"
                        ))
                    });
            }
        }
        Ok(true)
    }

    fn deliver_all_full_text_completions(
        &self,
        artifact_id: sillage_domain::ArtifactId,
        pending: &[Chunk],
    ) -> bool {
        for chunk in pending {
            if let Err(error) = Self::deliver_full_text_completion(
                &self.input_tx,
                FullTextIndexCompleted {
                    artifact_id,
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
                            EvidenceKind::FileSpan { path, .. }
                            | EvidenceKind::DocxParagraphSpan { path, .. } => Some(path.clone()),
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
    async fn quarantine_and_complete(&self, artifact_id: ArtifactId, pending: &[Chunk]) -> bool {
        let artifact = {
            let state = self.state.read().await;
            state.artifacts.get(&artifact_id).cloned()
        };
        let Some(artifact) = artifact else {
            return false;
        };
        let Some(hash) = artifact.content_hash else {
            return false;
        };
        if Self::send_input_blocking(
            &self.input_tx,
            DomainInput::ParserCompleted(sillage_domain::ParserResult {
                artifact_id,
                artifact_version_id: sillage_domain::ArtifactVersionId::new(artifact_id.value()),
                content_hash: hash,
                status: sillage_domain::ParseStatus::Quarantined,
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
        for chunk in pending {
            let completion = FullTextIndexCompleted {
                artifact_id,
                chunk_id: chunk.id,
            };
            if Self::deliver_full_text_completion(&self.input_tx, completion).is_err() {
                return false;
            }
        }
        true
    }

    /// Deliver a full-text completion after durable visibility.
    /// On `CapacityFull`, a detached task awaits channel capacity without
    /// re-running the writer transaction. A closed channel fails at shutdown.
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
        artifact_id: ArtifactId,
    ) -> Option<Vec<IndexedCard>> {
        let artifact_cards: Vec<IndexedCard> = {
            let state = self.state.read().await;
            state
                .cards
                .values()
                .filter(|card| card.artifact_id == artifact_id)
                .map(|card| IndexedCard {
                    artifact_id: card.artifact_id,
                    card_id: card.id,
                    title: card.title.clone(),
                    body: card.body.clone(),
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
