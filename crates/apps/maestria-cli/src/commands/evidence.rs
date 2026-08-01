use anyhow::{Context, Result, anyhow};
use maestria_core::{OpenChunkEvidenceInput, OpenEvidenceInput};
use maestria_daemon::evidence_open::{evidence_core_services, open_evidence_stores};
use maestria_domain::{ChunkId, EvidenceId};
use std::{path::PathBuf, time::Duration};

use crate::helpers;

pub fn run(instance_dir: PathBuf, evidence_id: Option<u64>, chunk_id: Option<u64>) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let stores = open_evidence_stores(&layout)?;
    let core = evidence_core_services(&stores);

    let output = if let Some(id) = evidence_id {
        helpers::retry_db_busy(Duration::from_secs(2), "opening evidence by id", || {
            core.open_evidence(OpenEvidenceInput {
                evidence_id: EvidenceId::new(id),
            })
            .map_err(anyhow::Error::from)
        })
        .context("open evidence by id")?
    } else if let Some(id) = chunk_id {
        helpers::retry_db_busy(Duration::from_secs(2), "opening chunk evidence", || {
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
