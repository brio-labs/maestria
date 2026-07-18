use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, OpenChunkEvidenceInput, OpenEvidenceInput};
use maestria_domain::{ChunkId, EvidenceId};
use maestria_parsers::ParserRegistry;
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use std::{path::PathBuf, thread, time::Duration};

use crate::helpers;

pub fn run(instance_dir: PathBuf, evidence_id: Option<u64>, chunk_id: Option<u64>) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let sqlite_store = SqliteStore::open(&layout.database_path)?;
    let blob_store = FsBlobStore::open(&layout.blobs_dir)?;
    let search_index = TantivyFullTextIndex::open_read_only(&layout.full_text_index_dir)?;
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
        vector_index: None,
        graph_index: None,
    });

    let output = if let Some(id) = evidence_id {
        retry_open_with_timeout(Duration::from_secs(2), || {
            core.open_evidence(OpenEvidenceInput {
                evidence_id: EvidenceId::new(id),
            })
            .map_err(anyhow::Error::from)
        })
        .context("open evidence by id")?
    } else if let Some(id) = chunk_id {
        retry_open_with_timeout(Duration::from_secs(2), || {
            core.open_chunk_evidence(OpenChunkEvidenceInput {
                chunk_id: ChunkId::new(id),
            })
            .map_err(anyhow::Error::from)
        })
        .context("open chunk evidence")?
    } else {
        return Err(anyhow!("provide --evidence-id or --chunk-id"));
    };

    println!(
        "artifact={} title={}",
        output.artifact.id, output.artifact.title
    );
    println!(
        "evidence={} {}",
        output.evidence.id,
        helpers::source_label(&output.evidence)
    );
    println!("excerpt={}", output.evidence.excerpt);
    Ok(())
}

fn retry_open_with_timeout<T>(
    timeout_budget: Duration,
    mut attempt: impl FnMut() -> Result<T>,
) -> Result<T> {
    let attempts = timeout_budget.as_millis().div_ceil(25).max(1);
    for attempt_number in 0..attempts {
        match attempt() {
            Ok(output) => return Ok(output),
            Err(error) if helpers::is_db_locked(&error) && attempt_number + 1 < attempts => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) if helpers::is_db_locked(&error) => {
                return Err(anyhow!(
                    "timed out while opening evidence due database lock: {error}"
                ));
            }
            Err(error) => return Err(error),
        }
    }
    Err(anyhow!(
        "timed out while opening evidence due database lock"
    ))
}
