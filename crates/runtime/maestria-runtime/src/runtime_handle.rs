use crate::runtime::{
    DomainApplicationResult, FeedbackError, MaestriaRuntime, RuntimeCommand, RuntimeHandle,
    RuntimeSubmissionError,
};
use maestria_domain::DomainInput;
use tokio::sync::{mpsc, oneshot};

impl RuntimeHandle {
    pub fn try_send_feedback(&self, input: DomainInput) -> Result<(), FeedbackError> {
        self.input_tx.try_send(input).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => FeedbackError::CapacityFull,
            mpsc::error::TrySendError::Closed(_) => FeedbackError::RuntimeShutdown,
        })
    }

    pub fn feedback_sender(&self) -> mpsc::Sender<DomainInput> {
        self.input_tx.clone()
    }

    /// Submit an application-bound domain command and await correlated domain
    /// acceptance plus complete effect-batch admission.
    pub async fn submit(
        &self,
        input: DomainInput,
    ) -> Result<DomainApplicationResult, RuntimeSubmissionError> {
        let correlation_id = self
            .next_command_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(RuntimeCommand {
                correlation_id,
                input,
                reply: reply_tx,
            })
            .await
            .map_err(|_| RuntimeSubmissionError::RuntimeShutdown)?;
        reply_rx
            .await
            .map_err(|_| RuntimeSubmissionError::RuntimeShutdown)?
    }
}

impl MaestriaRuntime {
    pub fn handle(&self) -> RuntimeHandle {
        RuntimeHandle {
            input_tx: self.input_tx.clone(),
            command_tx: self.command_tx.clone(),
            next_command_id: std::sync::Arc::clone(&self.next_command_id),
        }
    }

    pub fn with_graceful_shutdown(mut self) -> Self {
        self.config.drain_effects_on_shutdown = true;
        self
    }

    pub async fn snapshot_state(&self) -> maestria_domain::KernelState {
        self.state.read().await.clone()
    }

    /// Allocate a claim ID and a memory-candidate ID through the
    /// runtime's configured `IdAllocator`.
    pub fn allocate_memory_proposal_ids(
        &self,
    ) -> Result<
        (maestria_domain::ClaimId, maestria_domain::MemoryCandidateId),
        maestria_ports::PortError,
    > {
        let claim_id = self.adapters.id_allocator.allocate_claim_id()?;
        let candidate_id = self.adapters.id_allocator.allocate_memory_candidate_id()?;
        Ok((claim_id, candidate_id))
    }
}
