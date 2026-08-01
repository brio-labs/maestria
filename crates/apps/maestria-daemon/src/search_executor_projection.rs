use anyhow::Result;
use maestria_domain::{ArtifactVersionId, DomainEvent, DomainEventEnvelope, KernelState};
use maestria_governance::scan_secrets;
use maestria_ports::{FullTextIndex, IndexedCard};
use maestria_search_tantivy::TantivyFullTextIndex;
use std::collections::{BTreeMap, BTreeSet};

/// Project the active artifact versions from the domain event log.
///
/// `ParserStarted` marks the current version of a source path; a later
/// `SourceBecameStale` removes the path. The result drives
/// `CurrentVersionFilter` so stale versions never surface in retrieval.
pub(crate) fn reconcile_active_versions(
    events: &[DomainEventEnvelope],
) -> BTreeSet<ArtifactVersionId> {
    let mut latest_by_path = BTreeMap::new();
    for envelope in events {
        match &envelope.event {
            DomainEvent::ParserStarted {
                artifact_id,
                source_path,
                ..
            } => {
                latest_by_path.insert(
                    source_path.clone(),
                    ArtifactVersionId::new(artifact_id.value()),
                );
            }
            DomainEvent::SourceBecameStale { source_path, .. } => {
                latest_by_path.remove(source_path);
            }
            _ => {}
        }
    }
    latest_by_path.into_values().collect()
}

pub(super) fn ensure_search_index(
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
