use crate::types::*;
use std::sync::Arc;

impl KernelState {
    pub(super) fn replay_source_became_stale(
        &mut self,
        artifact_id: ArtifactId,
        source_path: &str,
    ) {
        let source_key = SourceIdentityKey::try_from(source_path.to_owned()).ok();
        let active = source_key
            .as_ref()
            .and_then(|key| self.active_sources.get(key))
            .copied();
        if active.is_some() && active != Some(artifact_id) {
            // A newer parse owns the path; this tombstone retires its predecessor.
            return;
        }
        Arc::make_mut(&mut self.stale_sources).insert(source_path.to_owned());
        if active == Some(artifact_id)
            && let Some(key) = source_key
        {
            Arc::make_mut(&mut self.active_sources).remove(&key);
        }
    }
}
