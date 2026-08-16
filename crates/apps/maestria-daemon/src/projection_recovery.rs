use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use anyhow::{Context, Result};
use maestria_domain::{ChunkId, EvidenceKind, KernelState};
use maestria_governance::scan_secrets;
use maestria_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EmbeddingProvider, EmbeddingRequest,
    EvidenceRepository, FullTextIndex, GraphIndex, IndexedCard, IndexedChunk, IndexedLexicalCard,
    IndexedLexicalChunk, VectorEmbedding, VectorIndex,
};
use maestria_storage_sqlite::SqliteStore;
/// Reconcile projection repositories from replayed domain truth.
///
/// After `load_kernel_state` replays the event log, this helper first removes
/// parent artifact rows, artifact mappings, and child projection rows whose IDs
/// are absent from the replayed state. It then idempotently upserts every
/// artifact, chunk, and card, and unconditionally replaces every evidence row
/// from the replayed state into the SQLite projection tables. Evidence uses
/// `replace` so a valid replayed row overwrites a stale, malformed, or partial
/// row from a prior crash without tripping a `Conflict` error.
///
/// Projection repair never emits domain events and never changes event truth. Startup recovery can
/// then search/open evidence even if the previous process crashed after event append but before a
/// projection write.
pub fn reconcile_projections(state: &KernelState, store: &SqliteStore) -> Result<()> {
    let artifact_ids: BTreeSet<_> = state.artifacts.keys().copied().collect();
    let chunk_ids: BTreeSet<_> = state.chunks.keys().copied().collect();
    let card_ids: BTreeSet<_> = state.cards.keys().copied().collect();
    let evidence_ids: BTreeSet<_> = state.evidences.keys().copied().collect();
    store
        .remove_stale_projection_rows(&artifact_ids, &chunk_ids, &card_ids, &evidence_ids)
        .context("remove stale projection rows")?;
    for artifact in state.artifacts.values() {
        ArtifactRepository::put(store, artifact.clone())
            .with_context(|| format!("put artifact {}", artifact.id))?;
    }
    for chunk in state.chunks.values() {
        ChunkRepository::put(store, chunk.clone())
            .with_context(|| format!("put chunk {}", chunk.id))?;
    }
    for card in state.cards.values() {
        CardRepository::put(store, card.clone())
            .with_context(|| format!("put card {}", card.id))?;
    }
    for evidence in state.evidences.values() {
        EvidenceRepository::replace(store, evidence.clone())
            .with_context(|| format!("replace evidence {}", evidence.id))?;
    }
    Ok(())
}

/// Repair missing full-text projection entries from replayed domain truth.
///
/// Full-text rows are derived data. Reindexing each current, retrieval-eligible
/// chunk and card is idempotent: adapters replace entries by their stable
/// artifact/entity key. Stale rows remain harmless because every retrieval
/// path pre-filters against current artifact state before scoring.
pub fn reconcile_full_text_projection(
    state: &KernelState,
    index: &(dyn FullTextIndex + Send + Sync),
) -> Result<()> {
    let source_paths: BTreeMap<_, _> = state
        .evidences
        .values()
        .filter_map(|evidence| match &evidence.kind {
            EvidenceKind::FileSpan { path, .. } => Some((evidence.artifact_id, path.clone())),
            _ => None,
        })
        .collect();

    let chunks: Vec<_> = state
        .chunks
        .values()
        .filter(|chunk| {
            state
                .artifacts
                .get(&chunk.artifact_id)
                .is_some_and(|artifact| artifact.security.retrieval_allowed())
                && scan_secrets(&chunk.text).is_clean()
        })
        .map(|chunk| {
            (
                IndexedChunk {
                    artifact_id: chunk.artifact_id,
                    chunk_id: chunk.id,
                    text: chunk.text.clone(),
                },
                source_paths.get(&chunk.artifact_id).cloned(),
            )
        })
        .collect();
    let cards: Vec<_> = state
        .cards
        .values()
        .filter(|card| {
            state
                .artifacts
                .get(&card.artifact_id)
                .is_some_and(|artifact| artifact.security.retrieval_allowed())
                && scan_secrets(&card.title).is_clean()
                && scan_secrets(&card.body).is_clean()
        })
        .map(|card| {
            (
                IndexedCard {
                    artifact_id: card.artifact_id,
                    card_id: card.id,
                    title: card.title.clone(),
                    body: card.body.clone(),
                },
                source_paths.get(&card.artifact_id).cloned(),
            )
        })
        .collect();

    if !chunks.is_empty() {
        index
            .index_chunks(chunks.iter().map(|(chunk, _)| chunk.clone()).collect())
            .context("index full-text chunks")?;
    }
    if !cards.is_empty() {
        index
            .index_cards(cards.iter().map(|(card, _)| card.clone()).collect())
            .context("index full-text cards")?;
    }
    index_lexical_metadata(index, &chunks, &cards)
}

fn index_lexical_metadata(
    index: &(dyn FullTextIndex + Send + Sync),
    chunks: &[(IndexedChunk, Option<String>)],
    cards: &[(IndexedCard, Option<String>)],
) -> Result<()> {
    if !index.supports_lexical_metadata() {
        return Ok(());
    }
    let lexical_chunks: Vec<IndexedLexicalChunk> = chunks
        .iter()
        .map(|(chunk, path)| IndexedLexicalChunk {
            artifact_id: chunk.artifact_id,
            chunk_id: chunk.chunk_id,
            text: chunk.text.clone(),
            path: path.clone(),
            filename: path
                .as_deref()
                .and_then(|path| Path::new(path).file_name())
                .and_then(|name| name.to_str())
                .map(str::to_owned),
            symbol: None,
        })
        .collect();
    let lexical_cards: Vec<IndexedLexicalCard> = cards
        .iter()
        .map(|(card, path)| IndexedLexicalCard {
            artifact_id: card.artifact_id,
            card_id: card.card_id,
            title: card.title.clone(),
            body: card.body.clone(),
            path: path.clone(),
            filename: path
                .as_deref()
                .and_then(|path| Path::new(path).file_name())
                .and_then(|name| name.to_str())
                .map(str::to_owned),
            symbol: None,
        })
        .collect();
    if !lexical_chunks.is_empty() {
        index
            .index_lexical_chunks(lexical_chunks)
            .context("index lexical full-text chunks")?;
    }
    if !lexical_cards.is_empty() {
        index
            .index_lexical_cards(lexical_cards)
            .context("index lexical full-text cards")?;
    }
    Ok(())
}

/// Rebuild the graph projection from replayed, evidenced relations.
///
/// Graph storage is a disposable projection. Clearing and rebuilding it at
/// startup repairs rows lost after an event was appended but before its graph
/// effect completed, while unevidenced relations remain intentionally absent.
pub fn reconcile_graph_projection(state: &KernelState, graph: &impl GraphIndex) -> Result<()> {
    let relations = state
        .relations
        .values()
        .filter(|relation| relation.evidence_id.is_some())
        .cloned()
        .collect();
    graph
        .rebuild(relations)
        .context("rebuild graph projection from domain state")?;
    Ok(())
}
/// Reconcile the vector projection from replayed chunks and the configured
/// embedding provider.
///
/// Vector rows are disposable and never determine domain truth. When
/// embeddings are disabled, reconciling with an empty set removes stale
/// rows. When embeddings are enabled, chunks whose indexed provenance
/// (content hash and generation identity) still matches the active profile
/// are kept without re-embedding, so startup recovery stays bounded even for
/// large corpora; only missing or stale chunks are embedded in stable
/// `ChunkId` order.
pub fn reconcile_vector_projection(
    state: &KernelState,
    vector_index: &(dyn VectorIndex + Send + Sync),
    embedding_provider: Option<&(dyn EmbeddingProvider + Send + Sync)>,
    embedding_model: Option<&str>,
) -> Result<()> {
    let eligible = eligible_chunks(state)?;
    let expected = eligible
        .iter()
        .map(|(chunk_id, _)| *chunk_id)
        .collect::<Vec<_>>();
    let embeddings = match (embedding_provider, embedding_model) {
        (None, None) => Vec::new(),
        (Some(provider), Some(model)) if !model.trim().is_empty() => {
            let identity = provider.identity().ok_or_else(|| {
                anyhow::anyhow!("vector projection recovery provider has no identity")
            })?;
            let existing = vector_index
                .indexed_embedding_keys()
                .context("read existing vector projection keys")?
                .into_iter()
                .map(|key| (key.chunk_id, key))
                .collect::<BTreeMap<_, _>>();
            embed_missing_chunks(state, provider, model, &identity, &eligible, &existing)?
        }
        (Some(_), Some(_)) => {
            return Err(anyhow::anyhow!(
                "vector projection recovery requires a non-empty embedding model"
            ));
        }
        (Some(_), None) => {
            return Err(anyhow::anyhow!(
                "vector projection recovery has an embedding provider but no model"
            ));
        }
        (None, Some(_)) => {
            return Err(anyhow::anyhow!(
                "vector projection recovery has an embedding model but no provider"
            ));
        }
    };
    vector_index
        .reconcile_projection(embeddings, &expected)
        .context("reconcile vector projection from domain state")?;
    Ok(())
}

/// Chunks eligible for dense projection: retrieval-allowed artifacts with
/// clean secret scans, paired with their content hash.
fn eligible_chunks(state: &KernelState) -> Result<Vec<(ChunkId, String)>> {
    let mut eligible = Vec::new();
    for chunk in state.chunks.values() {
        let artifact_allowed = state
            .artifacts
            .get(&chunk.artifact_id)
            .is_some_and(|artifact| artifact.security.retrieval_allowed());
        if !artifact_allowed || !scan_secrets(&chunk.text).is_clean() {
            continue;
        }
        // The indexed provenance identity is the embedded chunk text, not the
        // artifact hash: every writer (runtime effect handler and projection
        // recovery) stores `content_hash(chunk.text)`, so the skip must
        // compare the same identity or every prepare re-embeds the corpus.
        let content_hash =
            maestria_domain::ContentHash::new(maestria_domain::content_hash(chunk.text.as_bytes()))
                .map_err(|error| {
                    anyhow::anyhow!(
                        "computed content hash for chunk {} is invalid: {error}",
                        chunk.id
                    )
                })?;
        eligible.push((chunk.id, content_hash.as_str().to_owned()));
    }
    eligible.sort_by_key(|(chunk_id, _)| chunk_id.value());
    Ok(eligible)
}

/// Embed the eligible chunks whose indexed provenance no longer matches the
/// active identity; chunks already indexed with the same content hash and
/// generation identity are skipped.
fn embed_missing_chunks(
    state: &KernelState,
    provider: &(dyn EmbeddingProvider + Send + Sync),
    model: &str,
    identity: &maestria_ports::EmbeddingIdentity,
    eligible: &[(ChunkId, String)],
    existing: &BTreeMap<ChunkId, maestria_ports::IndexedEmbeddingKey>,
) -> Result<Vec<VectorEmbedding>> {
    let fingerprint = identity.fingerprint.encode();
    let generation_id = identity.generation_id.value().to_string();
    let mut embeddings = Vec::new();
    for (chunk_id, content_hash) in eligible {
        let already_indexed = existing.get(chunk_id).is_some_and(|key| {
            key.content_hash == *content_hash
                && key.generation_id == generation_id
                && key.representation == identity.representation.0
                && key.fingerprint == fingerprint
        });
        if already_indexed {
            continue;
        }
        let chunk = state.chunks.get(chunk_id).ok_or_else(|| {
            anyhow::anyhow!("chunk {} disappeared during projection recovery", chunk_id)
        })?;
        let response = match provider.embed(EmbeddingRequest {
            text: chunk.text.clone(),
            model: model.to_string(),
            kind: maestria_ports::EmbeddingInputKind::Document,
            identity: identity.clone(),
        }) {
            Ok(response) => response,
            Err(error) => {
                // The projection is best-effort: one unembeddable chunk
                // (oversized, provider hiccup, content the model rejects)
                // must not fail the whole startup reconcile. The chunk stays
                // unindexed and is retried on the next boot.
                tracing::warn!("embed chunk {} skipped: {error}", chunk_id);
                continue;
            }
        };
        if response.identity != *identity {
            return Err(anyhow::anyhow!(
                "embed chunk {} returned an incompatible generation identity",
                chunk_id
            ));
        }
        embeddings.push(VectorEmbedding {
            chunk_id: *chunk_id,
            vector: response.vector,
            provenance: maestria_ports::EmbeddingProvenance {
                content_hash: content_hash.clone(),
                identity: response.identity,
                provider_id: response.provider_id,
                model: response.model,
                model_version: response.model_version,
                disclosure: response.disclosure,
            },
        });
    }
    Ok(embeddings)
}

#[cfg(test)]
#[path = "projection_recovery_tests.rs"]
mod tests;
