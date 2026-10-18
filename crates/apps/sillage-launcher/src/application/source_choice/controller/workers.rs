use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::mpsc;

use super::super::search::run_query_worker;
use super::SourceChoiceController;
use super::actions::{ProofRequest, proof_worker};
use super::lifecycle::{SourceRequest, source_worker};
use super::presentation::run_state_publisher;
use crate::application::lock;

impl SourceChoiceController {
    pub(super) fn spawn_workers(
        self: &Arc<Self>,
        requests: mpsc::Receiver<SourceRequest>,
        proofs: mpsc::Receiver<ProofRequest>,
    ) {
        let mut workers = Vec::with_capacity(4);
        let state = Arc::clone(&self.state);
        let launcher_state = Arc::clone(&self.launcher_state);
        let frontend = Arc::clone(&self.frontend);
        let ui = self.ui.clone();
        let folder_search_setup = Arc::clone(&self.folder_search_setup);
        workers.push(self.runtime.spawn(async move {
            source_worker(
                state,
                launcher_state,
                frontend,
                ui,
                folder_search_setup,
                requests,
            )
            .await
        }));

        let state = Arc::clone(&self.state);
        let frontend = Arc::clone(&self.frontend);
        let ui = self.ui.clone();
        let search_state = Arc::clone(&state);
        workers.push(
            self.runtime
                .spawn(async move { run_query_worker(search_state, frontend, ui).await }),
        );

        let state = Arc::clone(&self.state);
        let frontend = Arc::clone(&self.frontend);
        let launcher_state = Arc::clone(&self.launcher_state);
        let ui = self.ui.clone();
        workers.push(
            self.runtime.spawn(async move {
                proof_worker(state, frontend, launcher_state, ui, proofs).await
            }),
        );

        let state = Arc::clone(&self.state);
        let controller = Arc::clone(self);
        let publisher = Arc::clone(self);
        workers.push(self.runtime.spawn(async move {
            run_state_publisher(
                state,
                move || controller.stopping.load(Ordering::Acquire),
                move || publisher.publish(),
            )
            .await;
            Ok(())
        }));

        *lock(&self.workers) = workers;
    }
}
