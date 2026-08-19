use crate::types::*;

impl KernelState {
    pub(super) fn replay_source_became_stale(
        &mut self,
        artifact_id: ArtifactId,
        source_path: &str,
    ) {
        let path = source_path.to_string();
        self.stale_sources.insert(path.clone());
        if let Ok(key) = SourceIdentityKey::try_from(path)
            && self.active_sources.get(&key) == Some(&artifact_id)
        {
            self.active_sources.remove(&key);
        }
    }
}
