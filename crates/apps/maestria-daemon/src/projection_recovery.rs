use anyhow::{Context, Result};
use maestria_domain::KernelState;
use maestria_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EvidenceRepository, GraphIndex,
};
use maestria_storage_sqlite::SqliteStore;

/// Reconcile projection repositories from replayed domain truth.
///
/// After `load_kernel_state` replays the event log, this helper idempotently upserts every artifact,
/// chunk, and card, and unconditionally replaces every evidence row from the replayed state into
/// the SQLite projection tables. Evidence uses `replace` so a valid replayed row overwrites a
/// stale, malformed, or partial row from a prior crash without tripping a `Conflict` error.
///
/// Projection repair never emits domain events and never changes event truth. Startup recovery can
/// then search/open evidence even if the previous process crashed after event append but before a
/// projection write.
pub fn reconcile_projections(state: &KernelState, store: &SqliteStore) -> Result<()> {
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
