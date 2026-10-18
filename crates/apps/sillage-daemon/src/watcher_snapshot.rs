use crate::search_executor::SearchRuntime;
use sillage_runtime::RuntimeHandle;

pub(super) enum SnapshotReadiness {
    Ready,
    Pending,
    Failed(String),
}

pub(super) struct SnapshotRefreshState {
    search_runtime: SearchRuntime,
    runtime_handle: RuntimeHandle,
    attempted_revision: Option<i64>,
    failure: Option<(i64, String)>,
}

impl SnapshotRefreshState {
    pub(super) fn new(search_runtime: SearchRuntime, runtime_handle: RuntimeHandle) -> Self {
        Self {
            search_runtime,
            runtime_handle,
            attempted_revision: None,
            failure: None,
        }
    }

    pub(super) async fn is_ready_for_current(
        &self,
        pending_deliveries: bool,
        pending_removals: bool,
    ) -> bool {
        if pending_deliveries || pending_removals || self.runtime_handle.has_pending_parsers().await
        {
            return false;
        }
        let Ok(revision) = self.search_runtime.searchable_source_revision() else {
            return false;
        };
        self.search_runtime
            .interactive_snapshot_is_current(revision)
    }

    pub(super) async fn prepare_if_ready(
        &mut self,
        pending_deliveries: bool,
        pending_removals: bool,
    ) -> SnapshotReadiness {
        let current_revision = match self.search_runtime.searchable_source_revision() {
            Ok(revision) => revision,
            Err(error) => {
                return SnapshotReadiness::Failed(format!(
                    "read searchable source revision: {error:#}"
                ));
            }
        };
        if self
            .failure
            .as_ref()
            .is_some_and(|(revision, _)| *revision != current_revision)
        {
            self.failure = None;
        }
        let cached_current = self
            .search_runtime
            .interactive_snapshot_is_current(current_revision);
        if cached_current {
            self.failure = None;
        } else if let Some((revision, error)) = &self.failure
            && *revision == current_revision
        {
            return SnapshotReadiness::Failed(error.clone());
        }

        if pending_deliveries || pending_removals || self.runtime_handle.has_pending_parsers().await
        {
            return SnapshotReadiness::Pending;
        }
        if cached_current {
            return self.confirm_cached_snapshot(current_revision).await;
        }
        self.prepare_snapshot(current_revision).await
    }

    async fn confirm_cached_snapshot(&mut self, current_revision: i64) -> SnapshotReadiness {
        let latest_revision = match self.search_runtime.searchable_source_revision() {
            Ok(revision) => revision,
            Err(error) => {
                return self.record_failure(
                    current_revision,
                    format!("recheck searchable source revision: {error:#}"),
                );
            }
        };
        if latest_revision != current_revision
            || !self
                .search_runtime
                .interactive_snapshot_is_current(latest_revision)
        {
            return SnapshotReadiness::Pending;
        }
        SnapshotReadiness::Ready
    }

    async fn prepare_snapshot(&mut self, current_revision: i64) -> SnapshotReadiness {
        if self.attempted_revision == Some(current_revision) {
            return self
                .failure
                .as_ref()
                .filter(|(revision, _)| *revision == current_revision)
                .map_or(SnapshotReadiness::Pending, |(_, error)| {
                    SnapshotReadiness::Failed(error.clone())
                });
        }

        self.attempted_revision = Some(current_revision);
        let search_runtime = self.search_runtime.clone();
        let preparation =
            tokio::task::spawn_blocking(move || search_runtime.prepare_interactive_snapshot())
                .await;
        let (prepared_revision, ready) = match preparation {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(error)) => {
                return self.record_failure(
                    current_revision,
                    format!("prepare interactive source snapshot: {error:#}"),
                );
            }
            Err(error) => {
                return self.record_failure(
                    current_revision,
                    format!("join interactive source snapshot preparation: {error}"),
                );
            }
        };
        if !ready {
            if prepared_revision == current_revision {
                return self.record_failure(
                    current_revision,
                    "interactive source snapshot cache did not reach the current revision"
                        .to_owned(),
                );
            }
            self.failure = None;
            return SnapshotReadiness::Pending;
        }

        let latest_revision = match self.search_runtime.searchable_source_revision() {
            Ok(revision) => revision,
            Err(error) => {
                return self.record_failure(
                    current_revision,
                    format!("recheck searchable source revision: {error:#}"),
                );
            }
        };
        if latest_revision != prepared_revision
            || !self
                .search_runtime
                .interactive_snapshot_is_current(latest_revision)
            || self.runtime_handle.has_pending_parsers().await
        {
            self.failure = None;
            return SnapshotReadiness::Pending;
        }
        self.failure = None;
        SnapshotReadiness::Ready
    }

    fn record_failure(&mut self, revision: i64, error: String) -> SnapshotReadiness {
        tracing::warn!(
            revision,
            error = %error,
            "interactive source snapshot preparation failed"
        );
        self.failure = Some((revision, error.clone()));
        SnapshotReadiness::Failed(error)
    }
}
