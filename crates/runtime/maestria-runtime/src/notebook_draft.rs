use crate::config::EffectExecutionContext;
use crate::persistence_barrier;
use maestria_domain::{
    ContentHash, DomainEvent, DomainInput, NotebookDraftBlobRequest, NotebookDraftBlobStored,
};
use tokio_util::sync::CancellationToken;

impl EffectExecutionContext {
    /// Persists draft bytes, verifies their content identity, and submits the
    /// correlated domain completion. Cancellation before the event barrier
    /// returns failure and leaves no visible draft event.
    pub(crate) async fn handle_persist_notebook_draft_blob(
        &self,
        request: NotebookDraftBlobRequest,
    ) -> bool {
        let bytes = request.body.as_bytes().to_vec();
        let expected_hash = match ContentHash::new(maestria_domain::content_hash(&bytes)) {
            Ok(hash) => hash,
            Err(error) => {
                tracing::error!(%error, "draft body hash construction failed");
                return self.fail_notebook_draft(&request, error.to_string());
            }
        };
        let blob_id = match self.adapters.blob_store.put(bytes.clone()) {
            Ok(blob_id) => blob_id,
            Err(error) => {
                tracing::error!(%error, "draft blob persistence failed");
                return self.fail_notebook_draft(&request, error.to_string());
            }
        };
        let stored = match self.adapters.blob_store.get(blob_id) {
            Ok(stored) => stored,
            Err(error) => {
                tracing::error!(%error, "draft blob verification read failed");
                return self.fail_notebook_draft(&request, error.to_string());
            }
        };
        if stored != bytes || maestria_domain::content_hash(&stored) != expected_hash.as_str() {
            tracing::error!("draft blob verification mismatch");
            return self.fail_notebook_draft(&request, "draft blob verification mismatch");
        }
        let notebook_id = request.notebook_id;
        let draft_id = request.draft_id;
        let expected_revision = request.expected_revision;
        let completion = DomainInput::NotebookDraftBlobStored(NotebookDraftBlobStored {
            notebook_id,
            draft_id,
            expected_revision,
            title: request.title.clone(),
            blob_id,
            content_hash: expected_hash.clone(),
            citations: request.citations.clone(),
            correlation_id: request.correlation_id,
        });
        if Self::send_input(&self.input_tx, completion, "notebook draft blob stored").is_err() {
            return self.fail_notebook_draft(&request, "runtime input channel unavailable");
        }
        let timeout = self
            .default_effect_timeout
            .min(std::time::Duration::from_secs(30));
        let persisted = persistence_barrier::wait_for_event(
            &*self.adapters.event_log,
            timeout,
            &CancellationToken::new(),
            "notebook draft persistence barrier",
            move |envelope| {
                matches!(
                    &envelope.event,
                    DomainEvent::NotebookDraftSaved {
                        notebook_id: event_notebook_id,
                        draft_id: event_draft_id,
                        revision,
                        body_blob,
                        body_hash,
                        ..
                    } if *event_notebook_id == notebook_id
                        && *body_blob == blob_id
                        && *body_hash == expected_hash
                        && draft_id.is_none_or(|id| id == *event_draft_id)
                        && expected_revision.is_none_or(|expected| {
                            expected.increment().is_ok_and(|next| next == *revision)
                        })
                )
            },
        )
        .await;
        if persisted {
            true
        } else {
            self.fail_notebook_draft(&request, "notebook draft event persistence failed")
        }
    }
    fn fail_notebook_draft(
        &self,
        request: &NotebookDraftBlobRequest,
        reason: impl Into<String>,
    ) -> bool {
        let Some(correlation_id) = request.correlation_id else {
            return false;
        };
        let input = DomainInput::NotebookDraftBlobStoreFailed(
            maestria_domain::NotebookDraftBlobStoreFailed {
                correlation_id,
                reason: reason.into(),
            },
        );
        Self::send_input(&self.input_tx, input, "notebook draft blob failure").is_ok()
    }
}
