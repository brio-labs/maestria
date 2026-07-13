use maestria_domain::{ArtifactId, DomainInput, KernelState, StartFullTextIndex};
use std::collections::BTreeSet;

pub fn pending_start_full_text(state: &KernelState) -> Vec<DomainInput> {
    let mut artifacts: BTreeSet<ArtifactId> = BTreeSet::new();
    for chunk_id in &state.pending_full_text {
        if let Some(chunk) = state.chunks.get(chunk_id) {
            // Skip artifacts that have a pending parser — the resumed
            // parser flow owns completion, evidence, and index ordering
            // and emits its own StartFullTextIndex afterward.  Issuing a
            // separate StartFullTextIndex here could make chunks terminal
            // before resumed evidence is recorded.
            if state.pending_parsers.contains_key(&chunk.artifact_id) {
                continue;
            }
            artifacts.insert(chunk.artifact_id);
        }
    }
    artifacts
        .into_iter()
        .map(|artifact_id| DomainInput::StartFullTextIndex(StartFullTextIndex { artifact_id }))
        .collect()
}
