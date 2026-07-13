use anyhow::Result;
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, SearchInput};
use maestria_domain::IndexStatus;
use maestria_parsers::ParserRegistry;
use maestria_ports::{FullTextIndex, IndexedCard};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use std::path::PathBuf;

use crate::helpers;

pub fn run(instance_dir: PathBuf, query: String, limit: usize) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let sqlite_store = SqliteStore::open(&layout.database_path)?;
    let blob_store = FsBlobStore::open(&layout.blobs_dir)?;
    let search_index = TantivyFullTextIndex::open(&layout.full_text_index_dir)?;
    if search_index.needs_card_rebuild()? {
        let state = maestria_daemon::load_kernel_state(&layout)?;
        let cards = state
            .cards
            .values()
            .filter(|card| {
                state
                    .artifacts
                    .get(&card.artifact_id)
                    .is_some_and(|artifact| artifact.index_status == IndexStatus::Indexed)
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
    }
    let parser = ParserRegistry::with_defaults();
    let core = CoreServices::new(CorePorts {
        artifacts: &sqlite_store,
        chunks: &sqlite_store,
        cards: &sqlite_store,
        evidence: &sqlite_store,
        events: &sqlite_store,
        parser: &parser,
        search_index: &search_index,
        blobs: &blob_store,
    });

    let output = core.search(SearchInput { query, limit })?;
    let pack = output.pack;

    // Card rows first, then chunk rows.
    for card_hit in &pack.cards {
        println!(
            "card score={} artifact={} card={} title={} body={}",
            card_hit.score,
            card_hit.artifact.id,
            card_hit.card.id,
            card_hit.card.title,
            card_hit.card.body,
        );
    }

    for hit in &pack.chunks {
        let source = helpers::source_label(&hit.evidence);
        println!(
            "score={} artifact={} chunk={} evidence={} {} snippet={}",
            hit.score,
            hit.artifact.id,
            hit.chunk.id,
            hit.evidence.id,
            source,
            hit.chunk
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    Ok(())
}
