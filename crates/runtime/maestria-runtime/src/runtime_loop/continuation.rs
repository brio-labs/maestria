//! Deferred approval-continuation application state machine.
//!
//! When an `ApprovalResolved` input resumes a pending model-agent proposal,
//! the resumed application may need to be deferred until the harness feedback
//! for the same run completes. This module owns that deferral lifecycle
//! (resume, merge, defer, complete) as a separate concern from the command
//! loop that drives it.

use maestria_domain::{
    DomainInput, HarnessRunId, ModelAgentProposalExecution, ModelAgentProposalRequest,
};
use tokio::sync::{mpsc, oneshot};

use crate::runtime::{
    DomainApplicationResult, EffectPreparation, MaestriaRuntime, PendingApplication,
    RuntimeCommand, RuntimeSubmissionError,
};

use super::ApplicationOutcome;

impl MaestriaRuntime {
    pub(crate) async fn process_inline_approval_continuation(
        &self,
        proposal: ModelAgentProposalRequest,
        correlation_id: Option<u64>,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> (
        bool,
        Option<Result<DomainApplicationResult, RuntimeSubmissionError>>,
    ) {
        let input = DomainInput::ModelAgentProposalResumed(proposal);
        let Some(correlation_id) = correlation_id else {
            let keep_running =
                Box::pin(self.process_input(input, None, effect_tx, shutdown_token)).await;
            return (keep_running, None);
        };
        let (reply, result) = oneshot::channel();
        let command = RuntimeCommand {
            correlation_id,
            effect_preparation: EffectPreparation::BeforeReply,
            reply,
        };
        let keep_running =
            Box::pin(self.process_input(input, Some(command), effect_tx, shutdown_token)).await;
        let result = match result.await {
            Ok(result) => result,
            Err(_) => Err(RuntimeSubmissionError::RuntimeShutdown),
        };
        (keep_running, Some(result))
    }

    pub(crate) async fn merge_inline_approval_continuation(
        &self,
        proposal: Option<ModelAgentProposalRequest>,
        should_resume: bool,
        command: &mut Option<RuntimeCommand>,
        outcome: &mut ApplicationOutcome,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> Result<Option<HarnessRunId>, bool> {
        let Some(proposal) = proposal.filter(|proposal| {
            should_resume
                && matches!(
                    proposal.execution,
                    ModelAgentProposalExecution::ApprovalContinuation { .. }
                )
        }) else {
            return Ok(None);
        };
        let run_id = proposal.run_id;
        let correlation_id = command.as_ref().map(|command| command.correlation_id);
        let (keep_running, continuation) = self
            .process_inline_approval_continuation(
                proposal,
                correlation_id,
                effect_tx,
                shutdown_token,
            )
            .await;
        if let Some(continuation) = continuation {
            match continuation {
                Ok(result) => {
                    outcome.effects_admitted = outcome
                        .effects_admitted
                        .saturating_add(result.effects_admitted);
                    outcome.events.extend(result.events);
                }
                Err(error) => {
                    if let Some(command) = command.take() {
                        super::deliver_reply(command.reply, Err(error));
                    }
                    return Err(keep_running);
                }
            }
        }
        if keep_running {
            return Ok(Some(run_id));
        }
        if let Some(command) = command.take() {
            let _ = command
                .reply
                .send(Err(RuntimeSubmissionError::RuntimeShutdown));
        }
        Err(false)
    }

    pub(crate) fn defer_application(
        &self,
        run_id: HarnessRunId,
        command: RuntimeCommand,
        outcome: ApplicationOutcome,
    ) -> bool {
        let application = PendingApplication {
            outcome: DomainApplicationResult {
                correlation_id: command.correlation_id,
                events: outcome.events,
                effects_admitted: outcome.effects_admitted,
            },
            command,
        };
        match self.pending_applications.lock() {
            Ok(mut pending) => {
                pending.insert(run_id, application);
                true
            }
            Err(_) => {
                tracing::error!("pending application lock poisoned");
                let _ = application
                    .command
                    .reply
                    .send(Err(RuntimeSubmissionError::RuntimeShutdown));
                false
            }
        }
    }

    pub(crate) fn complete_pending_application(
        &self,
        run_id: HarnessRunId,
        outcome: &mut ApplicationOutcome,
    ) {
        let application = match self.pending_applications.lock() {
            Ok(mut pending) => pending.remove(&run_id),
            Err(_) => {
                tracing::error!("pending application lock poisoned");
                None
            }
        };
        let Some(mut application) = application else {
            return;
        };
        application.outcome.effects_admitted = application
            .outcome
            .effects_admitted
            .saturating_add(outcome.effects_admitted);
        application.outcome.events.append(&mut outcome.events);
        super::deliver_reply(application.command.reply, Ok(application.outcome));
    }
}
