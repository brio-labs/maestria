use crate::{InstanceLifecycle, RecoveryQueue};
use anyhow::Result;
use maestria_core::InstanceLayout;
use maestria_domain::{DomainInput, KernelState};
use maestria_governance::AutonomyProfile;
use maestria_runtime::{DomainApplicationResult, RuntimeSubmissionError};

/// Ready lifecycle for one application mutation.
///
/// Construction acquires the instance lock, reconciles durable state, starts the runtime, and
/// applies queued recovery before returning. The session owns graceful effect draining and runtime
/// task joining.
pub struct MutationSession {
    lifecycle: InstanceLifecycle,
    recovery: RecoveryQueue,
}

impl MutationSession {
    /// Start a mutation session and apply startup recovery before accepting a new command.
    ///
    /// # Cancellation
    /// Cancellation before this future returns releases the instance lock and requests runtime
    /// shutdown. Recovery already accepted by the runtime may still reach durable state.
    pub async fn start(layout: InstanceLayout, profile: AutonomyProfile) -> Result<Self> {
        let mut lifecycle = InstanceLifecycle::start(layout, profile).await?;
        match lifecycle.queue_recovery().await {
            Ok(recovery) => Ok(Self {
                lifecycle,
                recovery,
            }),
            Err(error) => {
                let shutdown = lifecycle.shutdown().await;
                Err(combine_failures(error, shutdown))
            }
        }
    }

    pub fn state(&self) -> &KernelState {
        self.lifecycle.state()
    }

    /// Recovery work admitted before this session became ready.
    pub fn recovery(&self) -> &RecoveryQueue {
        &self.recovery
    }

    pub fn allocate_memory_proposal_ids(
        &self,
    ) -> Result<
        (maestria_domain::ClaimId, maestria_domain::MemoryCandidateId),
        maestria_ports::PortError,
    > {
        self.lifecycle
            .runtime_handle()
            .allocate_memory_proposal_ids()
    }

    /// Submit one correlated domain command.
    ///
    /// Success confirms domain acceptance and complete effect-batch admission. Dropping the future
    /// after channel acceptance does not cancel the server-side command; callers must inspect
    /// durable state before retrying an interrupted operation.
    pub async fn submit(
        &self,
        input: DomainInput,
    ) -> Result<DomainApplicationResult, RuntimeSubmissionError> {
        self.lifecycle.runtime_handle().submit(input).await
    }

    /// Finish the operation, drain admitted effects, and preserve operation and shutdown failures.
    ///
    /// # Cancellation
    /// Dropping this future requests shutdown through `InstanceLifecycle::drop`, but does not await
    /// effect draining or runtime task completion. Application callers should always await it.
    pub async fn finish<T>(self, operation: Result<T>) -> Result<T> {
        let shutdown = self.lifecycle.shutdown().await;
        combine_operation_and_shutdown(operation, shutdown)
    }
}

fn combine_failures(error: anyhow::Error, shutdown: Result<()>) -> anyhow::Error {
    match shutdown {
        Ok(()) => error,
        Err(shutdown_error) => error.context(format!(
            "lifecycle shutdown also failed: {shutdown_error:#}"
        )),
    }
}

fn combine_operation_and_shutdown<T>(operation: Result<T>, shutdown: Result<()>) -> Result<T> {
    match (operation, shutdown) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(shutdown_error)) => Err(error.context(format!(
            "lifecycle shutdown also failed: {shutdown_error:#}"
        ))),
    }
}

#[cfg(test)]
#[path = "mutation_session_tests.rs"]
mod tests;
