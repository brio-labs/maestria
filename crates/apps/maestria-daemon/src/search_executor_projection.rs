use anyhow::Result;
use maestria_domain::{ArtifactId, ArtifactVersionId, ContentHash, KernelState};
use maestria_governance::scan_secrets;
use maestria_ports::{FullTextIndex, IndexedCard};
use maestria_search_tantivy::TantivyFullTextIndex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Active artifact versions from the shared domain projection
/// ([`maestria_domain::active_source_versions`]); drives
/// `CurrentVersionFilter` so stale versions never surface in retrieval.
pub(crate) fn reconcile_active_versions(
    sources: &BTreeMap<PathBuf, (ArtifactId, ArtifactVersionId, ContentHash)>,
) -> BTreeSet<ArtifactVersionId> {
    sources.values().map(|(_, version, _)| *version).collect()
}

pub(crate) fn ensure_search_index(
    search_index: &TantivyFullTextIndex,
    state: &KernelState,
) -> Result<()> {
    if !search_index.needs_card_rebuild()? {
        return Ok(());
    }
    let cards: Vec<IndexedCard> = state
        .cards
        .values()
        .filter(|card| {
            state
                .artifacts
                .get(&card.artifact_id)
                .is_some_and(|artifact| {
                    artifact.index_status == maestria_domain::IndexStatus::Indexed
                })
                && scan_secrets(&card.title).is_clean()
                && scan_secrets(&card.body).is_clean()
        })
        .map(|card| IndexedCard {
            artifact_id: card.artifact_id,
            card_id: card.id,
            title: card.title.clone(),
            body: card.body.clone(),
        })
        .collect();
    search_index.index_cards(cards)?;
    search_index.complete_card_rebuild()?;
    Ok(())
}
