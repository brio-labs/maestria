//! Startup reconciliation watermarking.
//!
//! Durable projections are derived data. A snapshot of durable truth plus
//! every projection's observable state is pinned after a clean reconcile and
//! re-advanced when a runtime session ends cleanly; startup skips the
//! expensive reconciles only while that snapshot stays equal to freshly
//! computed state.

use anyhow::Context as _;
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::KernelState;
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;

use crate::projection_recovery::{
    reconcile_full_text_projection, reconcile_graph_projection, reconcile_projections,
};
use crate::vector_startup::reconcile_retrieval_generations;

/// Hash of the embedding configuration section for watermark comparison.
///
/// `EmbeddingConfig` is not `Serialize`; `Debug` covers every field and only
/// needs to be stable within one binary version.
pub(crate) fn embedding_config_hash(manifest: &InstanceManifest) -> String {
    match manifest.embeddings.as_ref() {
        Some(config) => maestria_core::content_hash(format!("{config:?}").as_bytes()),
        None => String::new(),
    }
}

/// Re-run every event-derived projection reconcile when durable truth or any
/// projection drifted since the last clean session; pin a fresh watermark on
/// success. Returns whether the stored watermark was already clean (drift
/// gates the vector and learned-sparse rebuilds at their call sites).
pub(crate) fn reconcile_after_drift(
    layout: &InstanceLayout,
    state: &mut KernelState,
    store: &SqliteStore,
    manifest: &InstanceManifest,
    embedding_hash: &str,
) -> anyhow::Result<bool> {
    let watermark_clean = match (
        store.projection_watermark(),
        snapshot(layout, Some(state), embedding_hash),
    ) {
        (Ok(Some(stored)), Some(snapshot)) => Some(stored) == Some(snapshot.to_string()),
        _ => false,
    };
    if watermark_clean {
        return Ok(true);
    }
    reconcile_retrieval_generations(layout, state, manifest)
        .with_context(|| "reconcile retrieval generations")?;
    reconcile_projections(state, store).with_context(|| "reconcile projection repositories")?;
    let search_index = crate::projection_open::open_full_text_index(layout, state, true, true)
        .with_context(|| "open full-text projection")?;
    reconcile_full_text_projection(state, &*search_index)
        .with_context(|| "reconcile full-text projection")?;
    drop(search_index);
    let graph_index = SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))
        .with_context(|| format!("open graph index {}", layout.graph_index_dir.display()))?;
    reconcile_graph_projection(state, &graph_index)
        .with_context(|| "reconcile graph projection")?;
    drop(graph_index);
    let watermark = snapshot(layout, Some(state), embedding_hash);
    let encoded = match &watermark {
        Some(value) => value.to_string(),
        None => String::new(),
    };
    if let Err(error) = store.set_projection_watermark(encoded.as_str()) {
        tracing::warn!(
            %error,
            "failed to persist projection reconciliation watermark; next start will reconcile again"
        );
    }
    Ok(false)
}

/// Pin the reconciliation watermark after a clean runtime session.
///
/// Best-effort: failures only cause the next start to reconcile again.
pub(crate) fn finalize(layout: &InstanceLayout, state_snapshot: Option<serde_json::Value>) {
    let Some(snapshot) = state_snapshot else {
        return;
    };
    let encoded = snapshot.to_string();
    if let Ok(store) = SqliteStore::open(&layout.database_path) {
        match store.projection_watermark() {
            // Already current: skip the write so read-only sessions that
            // transiently won the instance lock stay cheap.
            Ok(Some(stored)) if stored == encoded => {}
            Ok(_) => {
                if let Err(error) = store.set_projection_watermark(encoded.as_str()) {
                    tracing::warn!(
                        %error,
                        "failed to advance projection reconciliation watermark"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(%error, "failed to read reconciliation watermark for advance");
            }
        }
    }
}

/// Whether durable truth is unchanged since the last successful startup
/// reconciliation, making the expensive projection reconciles skippable.
///
/// The full reconciliation watermark snapshot: durable event-log position,
/// embedding-config hash, per-projection row counts, and the live lexical
/// index fingerprint plus document count.
///
/// A stored snapshot equal to a freshly computed one proves durable truth
/// AND every derived projection are unchanged since the last reconciliation,
/// so startup can skip it; any drift (new events, config change, external
/// projection tampering) forces repair from kernel truth.
pub(crate) fn snapshot(
    layout: &InstanceLayout,
    state: Option<&KernelState>,
    embedding_hash: &str,
) -> Option<serde_json::Value> {
    // Read-only on purpose: the snapshot runs on every durable-command
    // start, so it must not take a migration/write lock on the database. A
    // missing database or missing projection_meta table simply yields None
    // (dirty) and the writable dirty-path creates them.
    let store = SqliteStore::open_read_only(&layout.database_path).ok()?;
    let max_event_id = store.max_event_id().ok()?;
    let (artifacts, chunks, cards, evidences) = store.projection_row_counts().ok()?;
    let graph_relations = SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))
        .ok()?
        .relation_count()
        .ok()?;
    let vector_embeddings = SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db"))
        .ok()?
        .embedding_row_count()
        .ok()?;
    let lexical_index = TantivyFullTextIndex::open_read_only(&layout.full_text_index_dir).ok()?;
    if lexical_index.needs_card_rebuild().ok()? {
        return None;
    }
    let lexical_fingerprint = lexical_index.fingerprint().ok()?;
    // The replayed generation fingerprint must still match the live index:
    // catches content tampering that preserves the document count.
    if let Some(state) = state
        && state
            .index_generations
            .get_active(&maestria_domain::RepresentationName::new("lexical_text_v1"))
            .is_some_and(|generation| generation.fingerprint != lexical_fingerprint)
    {
        return None;
    }
    // Built without the `json!` macro: its internal expansion trips the
    // workspace-wide disallowed-method lint for `Result::unwrap`.
    let mut counts = serde_json::Map::new();
    counts.insert("artifacts".into(), artifacts.into());
    counts.insert("chunks".into(), chunks.into());
    counts.insert("cards".into(), cards.into());
    counts.insert("evidences".into(), evidences.into());
    counts.insert("graph_relations".into(), graph_relations.into());
    counts.insert("vector_embeddings".into(), vector_embeddings.into());
    counts.insert(
        "lexical_docs".into(),
        lexical_index.doc_count().ok()?.into(),
    );
    let mut snapshot = serde_json::Map::new();
    snapshot.insert("max_event_id".into(), max_event_id.into());
    snapshot.insert("embedding_hash".into(), embedding_hash.into());
    snapshot.insert("counts".into(), counts.into());
    snapshot.insert(
        "lexical_fingerprint".into(),
        lexical_fingerprint.encode().into(),
    );
    Some(serde_json::Value::Object(snapshot))
}
