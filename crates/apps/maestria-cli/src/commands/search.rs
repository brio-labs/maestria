use anyhow::{Context, Result};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, InstanceService, SearchInput};
use maestria_domain::{DomainInput, IndexStatus, LogicalTick, SearchExecutedInput};
use maestria_governance::AutonomyProfile;
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_parsers::ParserRegistry;
use maestria_ports::{
    EmbeddingProvider, EmbeddingRequest, FullTextIndex, GraphIndex, IndexedCard, VectorIndex,
    VectorSearchQuery,
};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;
use std::{fs, path::PathBuf, sync::Arc, time::Duration};

use crate::helpers;
pub async fn run(instance_dir: PathBuf, query: String, limit: usize) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let _instance_lock = maestria_daemon::acquire_instance_write_lock(&layout).await?;
    let normalized_query = query.trim().to_string();
    let search_layout = layout.clone();
    let search_query = normalized_query.clone();
    let pack = tokio::task::spawn_blocking(move || {
        compute_search_pack(&search_layout, &search_query, limit)
    })
    .await
    .map_err(|error| anyhow::anyhow!("search worker failed: {error}"))??;
    persist_search_audit(&layout, &pack, limit).await?;
    print_search_pack(&pack);
    Ok(())
}

fn compute_search_pack(
    layout: &maestria_core::InstanceLayout,
    query: &str,
    limit: usize,
) -> Result<maestria_core::EvidencePack> {
    let sqlite_store = SqliteStore::open(&layout.database_path)?;
    let blob_store = FsBlobStore::open(&layout.blobs_dir)?;
    let manifest_contents = fs::read_to_string(&layout.manifest_path)?;
    let manifest = InstanceService::parse_manifest(&manifest_contents)?;
    let embedding_provider: Option<Arc<dyn EmbeddingProvider>> =
        match manifest.embeddings.as_ref().filter(|config| config.enabled) {
            Some(config) => Some(Arc::new(
                maestria_embedding_openai::LocalHttpEmbeddingProvider::new(
                    &config.endpoint,
                    &config.model,
                    Some(config.dimensions),
                )?,
            )),
            None => None,
        };
    let vector_index = if embedding_provider.is_some() {
        match SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db")) {
            Ok(index) => Some(index),
            Err(error) => {
                eprintln!("vector projection unavailable; using full-text fallback: {error}");
                None
            }
        }
    } else {
        None
    };
    let graph_index = match SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db")) {
        Ok(index) => match maestria_daemon::load_kernel_state(layout)
            .and_then(|state| maestria_daemon::reconcile_graph_projection(&state, &index))
        {
            Ok(()) => Some(index),
            Err(error) => {
                eprintln!("graph projection unavailable; using retrieval-only search: {error}");
                None
            }
        },
        Err(error) => {
            eprintln!("graph projection unavailable; using retrieval-only search: {error}");
            None
        }
    };
    let search_index = TantivyFullTextIndex::open(&layout.full_text_index_dir)?;
    if search_index.needs_card_rebuild()? {
        let state = maestria_daemon::load_kernel_state(layout)?;
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
    let vector_query = embedding_provider.as_deref().and_then(|provider| {
        provider
            .embed(EmbeddingRequest {
                text: query.to_string(),
                model: "query".to_string(),
            })
            .ok()
            .map(|response| VectorSearchQuery {
                vector: response.vector,
                limit: limit as u32,
                provider_id: Some(response.provider_id),
                model: Some(response.model),
                model_version: Some(response.model_version),
            })
    });
    let core = CoreServices::new(CorePorts {
        artifacts: &sqlite_store,
        chunks: &sqlite_store,
        cards: &sqlite_store,
        evidence: &sqlite_store,
        events: &sqlite_store,
        parser: &parser,
        search_index: &search_index,
        blobs: &blob_store,
        vector_index: vector_index.as_ref().map(|index| index as &dyn VectorIndex),
        graph_index: graph_index.as_ref().map(|index| index as &dyn GraphIndex),
    });
    let input = SearchInput {
        query: query.to_string(),
        limit,
    };
    let output = match vector_query {
        Some(vector_query) => core.search_with_vector(input, vector_query)?,
        None => core.search(input)?,
    };
    Ok(output.pack)
}

async fn persist_search_audit(
    layout: &maestria_core::InstanceLayout,
    pack: &maestria_core::EvidencePack,
    limit: usize,
) -> Result<()> {
    let state =
        maestria_daemon::load_kernel_state(layout).context("load kernel state for search audit")?;
    let event_count_before = state.event_log.len();
    let (runtime, input_tx, input_rx, shutdown_token) =
        maestria_daemon::build_runtime(layout, state, AutonomyProfile::TrustedWorkspace)
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
            layout,
            event_count_before,
            &pack.query,
            limit,
            &pack.evidence_ids,
            Duration::from_secs(5),
        )
        .await
    }
    .await;
    shutdown_token.cancel();
    let join_result = runtime_task.await;
    result?;
    join_result.with_context(|| "runtime loop join failed")?;
    Ok(())
}

fn print_search_pack(pack: &maestria_core::EvidencePack) {
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
                    if event_count_before < state.event_log.len() {
                        let new_events = &state.event_log[event_count_before..];
                        if new_events.iter().any(|envelope| {
                            envelope.id.value() == event_count_before as u64 + 1
                                && matches!(
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
