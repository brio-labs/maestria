use anyhow::{Context, Result};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, SearchInput};
use maestria_domain::{DomainInput, IndexStatus, LogicalTick, SearchExecutedInput};
use maestria_governance::AutonomyProfile;
use maestria_parsers::ParserRegistry;
use maestria_ports::{FullTextIndex, IndexedCard};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use std::path::PathBuf;
use std::time::Duration;

use crate::helpers;
pub async fn run(instance_dir: PathBuf, query: String, limit: usize) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let normalized_query = query.trim().to_string();

    // ── Search computation phase (local stores, borrowed by CoreServices) ──
    let pack = {
        let sqlite_store = SqliteStore::open(&layout.database_path)?;
        let blob_store = FsBlobStore::open(&layout.blobs_dir)?;
        let search_index = TantivyFullTextIndex::open(&layout.full_text_index_dir)?;
        if search_index.needs_card_rebuild()? {
            let state = maestria_daemon::load_kernel_state(&layout)?;
            let cards: Vec<IndexedCard> = state
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
        let output = core.search(SearchInput {
            query: normalized_query.clone(),
            limit,
        })?;
        output.pack
    }; // All local stores and borrows are dropped here.
    let state = maestria_daemon::load_kernel_state(&layout)
        .context("load kernel state for search audit")?;
    let event_count_before = state.event_log.len();
    let (runtime, input_tx, input_rx, shutdown_token) =
        maestria_daemon::build_runtime(&layout, state, AutonomyProfile::TrustedWorkspace)
            .with_context(|| "build runtime for search audit")?;
    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    let audit_input = SearchExecutedInput {
        query: pack.query.clone(),
        limit,
        evidence_ids: pack.evidence_ids.clone(),
        at: LogicalTick::new(1),
    };

    let result = async {
        input_tx
            .send(DomainInput::SearchExecuted(audit_input))
            .await
            .map_err(|err| anyhow::anyhow!("failed to queue search audit input: {err}"))?;
        wait_for_search_executed_persistence(
            &layout,
            event_count_before,
            &pack.query,
            limit,
            &pack.evidence_ids,
            Duration::from_secs(5),
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    shutdown_token.cancel();
    let join_result = runtime_task.await;
    result?;
    join_result.with_context(|| "runtime loop join failed")?;

    // ── Print results only after successful audit persistence ──
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

/// Poll the event log until this search's SearchExecuted event is observed.
async fn wait_for_search_executed_persistence(
    layout: &maestria_core::InstanceLayout,
    event_count_before: usize,
    expected_query: &str,
    expected_limit: usize,
    expected_evidence_ids: &[maestria_domain::EvidenceId],
    timeout_budget: Duration,
) -> Result<()> {
    tokio::time::timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout) {
                Ok(state) => {
                    let new_events = state.event_log.get(event_count_before..).unwrap_or(&[]);
                    if new_events.iter().any(|envelope| {
                        matches!(
                            &envelope.event,
                            maestria_domain::DomainEvent::SearchExecuted {
                                query,
                                limit,
                                evidence_ids,
                                ..
                            } if query == expected_query
                                && *limit == expected_limit
                                && evidence_ids == expected_evidence_ids
                        )
                    }) {
                        return Ok(());
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) if helpers::is_db_locked(&error) => {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for search audit persistence"))?
}
