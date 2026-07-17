use anyhow::Result;
use std::path::PathBuf;

use crate::helpers;

pub fn run_index_generations(instance_dir: PathBuf) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let state = maestria_daemon::load_kernel_state(&layout)?;
    if state.index_generations.is_empty() {
        println!("index_generations=none");
        return Ok(());
    }
    for generation in state.index_generations.iter() {
        println!(
            "generation={} name={} lifecycle={:?} serveable={} snapshot={}",
            generation.id,
            generation.name.0,
            generation.lifecycle,
            state.index_generations.is_serveable(generation.id),
            generation.corpus_snapshot,
        );
        println!(
            "fingerprint provider={} model={} revision={} dimensions={} quantization={} artifact_hash={:?} query_template_hash={} document_template_hash={} preprocessing={}",
            generation.fingerprint.provider,
            generation.fingerprint.model,
            generation.fingerprint.revision,
            generation.fingerprint.dimensions,
            generation.fingerprint.quantization,
            generation.fingerprint.artifact_hash,
            generation.fingerprint.query_template_hash,
            generation.fingerprint.document_template_hash,
            generation.fingerprint.preprocessing_version,
        );
    }
    Ok(())
}
