use crate::runtime::{
    DomainApplicationResult, EffectPreparation, FeedbackError, MaestriaRuntime, RuntimeCommand,
    RuntimeHandle, RuntimeSubmissionError, RuntimeSubmissionPermit,
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

    /// Reserve bounded capacity for one correlated submission without consuming its input.
    ///
    /// # Cancellation
    /// If reservation is cancelled, the caller still owns the input. Once this method returns,
    /// calling [`RuntimeSubmissionPermit::submit`] accepts the input before awaiting its result.
    pub async fn reserve_submission(
        &self,
    ) -> Result<RuntimeSubmissionPermit, RuntimeSubmissionError> {
        let permit = self
            .command_tx
            .clone()
            .reserve_owned()
            .await
            .map_err(|_| RuntimeSubmissionError::RuntimeShutdown)?;
        let correlation_id = self
            .next_command_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(RuntimeSubmissionPermit {
            permit,
            correlation_id,
        })
    }

    /// Submit an application-bound domain command and await correlated domain
    /// acceptance plus complete effect-batch admission.
    ///
    /// # Cancellation
    /// Cancellation while reserving capacity leaves the input unsubmitted. Once
    /// reservation succeeds, the command is sent before this future awaits the
    /// correlated result; cancelling afterward leaves the runtime processing the
    /// command while its reply is discarded.
    pub async fn submit(
        &self,
        input: DomainInput,
    ) -> Result<DomainApplicationResult, RuntimeSubmissionError> {
        self.reserve_submission().await?.submit(input).await
    }
    /// Allocate a claim ID and memory-candidate ID through the configured durable allocator.
    pub fn allocate_memory_proposal_ids(
        &self,
    ) -> Result<
        (maestria_domain::ClaimId, maestria_domain::MemoryCandidateId),
        maestria_ports::PortError,
    > {
        let claim_id = self.id_allocator.allocate_claim_id()?;
        let candidate_id = self.id_allocator.allocate_memory_candidate_id()?;
        Ok((claim_id, candidate_id))
    }
}

impl RuntimeSubmissionPermit {
    /// Submit using previously reserved capacity.
    ///
    /// # Cancellation
    /// The command is sent synchronously before this future awaits the
    /// correlated result. Cancellation after polling reaches that send leaves
    /// the runtime processing the command while its reply is discarded.
    pub async fn submit(
        self,
        input: DomainInput,
    ) -> Result<DomainApplicationResult, RuntimeSubmissionError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.permit.send(RuntimeCommand {
            correlation_id: self.correlation_id,
            input,
            effect_preparation: EffectPreparation::Deferred,
            reply: reply_tx,
        });
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
            id_allocator: std::sync::Arc::clone(&self.adapters.id_allocator),
        }
    }

    pub fn with_graceful_shutdown(mut self) -> Self {
        self.config.drain_effects_on_shutdown = true;
        self
    }
}
