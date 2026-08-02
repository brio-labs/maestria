use anyhow::{Context, Result, anyhow};
use maestria_daemon::evidence_open::{open_chunk_evidence_scoped, open_evidence_scoped};
use std::path::PathBuf;

use crate::helpers;

pub fn run(instance_dir: PathBuf, evidence_id: Option<u64>, chunk_id: Option<u64>) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;

    // The scoped open is the single owner of instance read-scope and retrieval
    // policy enforcement (R48); the CLI surface must not bypass it.
    let output = if let Some(id) = evidence_id {
        helpers::retry_db_busy("opening evidence by id", || {
            open_evidence_scoped(&layout, id)
        })
        .context("open evidence by id")?
    } else if let Some(id) = chunk_id {
        helpers::retry_db_busy("opening chunk evidence", || {
            open_chunk_evidence_scoped(&layout, id)
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
