use std::sync::atomic::Ordering;

use tokio::sync::oneshot;

use super::SourceChoiceController;
use super::lifecycle::SourceRequest;
use crate::application::lock;

impl SourceChoiceController {
    pub(in crate::application) async fn shutdown(&self) -> Result<(), String> {
        if self.stopping.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.state.stop_new_admission();
        let (settled, wait) = oneshot::channel();
        let actor_result = match self.requests.send(SourceRequest::Shutdown(settled)).await {
            Ok(()) => match wait.await {
                Ok(result) => result,
                Err(_) => {
                    Err("The selected-source supervisor dropped its shutdown result".to_string())
                }
            },
            Err(_) => {
                Err("The selected-source supervisor stopped before shutdown settlement".to_string())
            }
        };
        lock(&self.proof_requests).take();
        let workers = std::mem::take(&mut *lock(&self.workers));
        let mut failure = actor_result.err();
        for worker in workers {
            match worker.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => append_shutdown_failure(&mut failure, error),
                Err(error) => append_shutdown_failure(
                    &mut failure,
                    format!("Selected-source worker failed to settle: {error}"),
                ),
            }
        }
        match self.state.retire_active().await {
            Ok(()) => self.state.mark_stopped(),
            Err(error) => append_shutdown_failure(&mut failure, error),
        }
        for error in self.state.take_terminal_operation_failures() {
            append_shutdown_failure(&mut failure, error);
        }

        failure.map_or(Ok(()), Err)
    }
}

fn append_shutdown_failure(failure: &mut Option<String>, message: String) {
    if let Some(failure) = failure {
        failure.push_str("; ");
        failure.push_str(&message);
    } else {
        *failure = Some(message);
    }
}
