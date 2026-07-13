use anyhow::{Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, OpenChunkEvidenceInput, OpenEvidenceInput};
use maestria_domain::{ChunkId, EvidenceId};
use maestria_parsers::ParserRegistry;
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use std::path::PathBuf;

use crate::helpers;

pub fn run(instance_dir: PathBuf, evidence_id: Option<u64>, chunk_id: Option<u64>) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let sqlite_store = SqliteStore::open(&layout.database_path)?;
    let blob_store = FsBlobStore::open(&layout.blobs_dir)?;
    let search_index = TantivyFullTextIndex::open(&layout.full_text_index_dir)?;
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

    let output = if let Some(id) = evidence_id {
        core.open_evidence(OpenEvidenceInput {
            evidence_id: EvidenceId::new(id),
        })?
    } else if let Some(id) = chunk_id {
        core.open_chunk_evidence(OpenChunkEvidenceInput {
            chunk_id: ChunkId::new(id),
        })?
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
