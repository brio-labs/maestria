use anyhow::Result;

use super::SearchRuntime;

impl SearchRuntime {
    pub(crate) fn searchable_source_revision(&self) -> Result<i64> {
        self.event_log
            .searchable_source_revision()
            .map_err(anyhow::Error::new)
    }

    pub(crate) fn interactive_snapshot_is_current(&self, revision: i64) -> bool {
        self.interactive_cache
            .read()
            .as_ref()
            .is_some_and(|snapshot| snapshot.revision == revision)
    }

    /// Prepare a snapshot for the latest published source revision off the
    /// interactive request path. `false` means publication raced a source
    /// change, so callers must not report indexing ready for this pass.
    pub(crate) fn prepare_interactive_snapshot(&self) -> Result<(i64, bool)> {
        let _ = self.interactive_snapshot()?;
        let revision = self.searchable_source_revision()?;
        Ok((revision, self.interactive_snapshot_is_current(revision)))
    }
}
