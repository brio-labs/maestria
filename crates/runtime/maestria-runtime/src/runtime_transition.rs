use crate::runtime::{MaestriaRuntime, RuntimeCommand, RuntimeSubmissionError};
use maestria_domain::{DomainError, DomainInput, KernelState, MaestriaEffect};

pub(crate) struct TransitionBarriers {
    pub(crate) proposal_event_id: Option<maestria_domain::EventId>,
    pub(crate) approval: Option<(maestria_domain::EventId, maestria_domain::ApprovalId, bool)>,
    pub(crate) validation_report_id: Option<maestria_domain::ValidationReportId>,
    pub(crate) durable_event_ids: Vec<maestria_domain::EventId>,
    pub(crate) durable_grant_event_ids:
        Vec<(maestria_domain::EventId, maestria_domain::GrantTokenDigest)>,
}

impl MaestriaRuntime {
    pub(crate) fn correlate_proposal(
        input: DomainInput,
        command: Option<&RuntimeCommand>,
    ) -> DomainInput {
        match input {
            DomainInput::ModelAgentProposalRequested(mut proposal) => {
                if let Some(command) = command {
                    proposal.correlation_id =
                        maestria_domain::CorrelationId::new(command.correlation_id);
                }
                DomainInput::ModelAgentProposalRequested(proposal)
            }
            other => other,
        }
    }

    pub(crate) fn completed_run_id(input: &DomainInput) -> Option<maestria_domain::HarnessRunId> {
        match input {
            DomainInput::ModelAgentProposalCompleted(result) => Some(result.run_id()),
            _ => None,
        }
    }

    pub(crate) fn approval_continuation(
        &self,
        input: &DomainInput,
    ) -> Result<Option<maestria_domain::ModelAgentProposalRequest>, String> {
        let DomainInput::ApprovalResolved(decision) = input else {
            return Ok(None);
        };
        let record = match self
            .adapters
            .approval_repo
            .find_by_id(decision.approval_id())
        {
            Ok(record) => record,
            Err(error) => {
                return Err(format!(
                    "look up approval {} for continuation: {error}",
                    decision.approval_id()
                ));
            }
        };
        let Some(record) = record else {
            return Ok(None);
        };
        crate::proposal_persistence::decode_pending_continuation(&record)
    }

    pub(crate) async fn boundary_error(&self, input: &DomainInput) -> Option<&'static str> {
        match input {
            DomainInput::ApprovalResolved(decision)
                if !self.check_approval_boundary(decision).await =>
            {
                Some("approval decision failed boundary validation")
            }
            DomainInput::CompleteTask(complete_input)
                if !self.check_completion_validation(complete_input).await =>
            {
                Some("task completion failed validation boundary")
            }
            DomainInput::HarnessRunCompleted(completion)
                if !self.check_harness_feedback_boundary(completion) =>
            {
                Some("harness completion failed journal boundary validation")
            }
            _ => None,
        }
    }

    pub(crate) fn harness_feedback(
        input: &DomainInput,
    ) -> Option<(maestria_domain::HarnessRunId, u64)> {
        match input {
            DomainInput::HarnessRunCompleted(completion) => {
                Some((completion.run_id, completion.generation))
            }
            _ => None,
        }
    }

    pub(crate) fn approval_barrier(
        input: &DomainInput,
        command: Option<&RuntimeCommand>,
    ) -> Option<(maestria_domain::ApprovalId, bool)> {
        match (input, command) {
            (DomainInput::ApprovalResolved(decision), Some(_)) => {
                Some((decision.approval_id(), decision.approved()))
            }
            _ => None,
        }
    }

    pub(crate) async fn stage_input(
        &self,
        input: DomainInput,
        has_approval_continuation: bool,
    ) -> Result<(KernelState, maestria_domain::KernelOutput, bool), DomainError> {
        let state = self.state.read().await;
        let mut candidate = state.clone();
        let should_resume_approval = matches!(
            &input,
            DomainInput::ApprovalResolved(decision)
                if has_approval_continuation
                    && !state.resolved_approvals.contains(&decision.approval_id())
        );
        drop(state);
        let output = candidate.apply_input(input)?;
        Ok((candidate, output, should_resume_approval))
    }

    pub(crate) fn transition_barriers(
        events: &[maestria_domain::DomainEventEnvelope],
        effects: &[MaestriaEffect],
        approval_barrier: Option<(maestria_domain::ApprovalId, bool)>,
        durable: bool,
    ) -> TransitionBarriers {
        let proposal_event_id = events.iter().find_map(|event| {
            matches!(
                &event.event,
                maestria_domain::DomainEvent::ModelAgentProposalRequested { .. }
            )
            .then_some(event.id)
        });
        let approval = approval_barrier.and_then(|(approval_id, approved)| {
            events.iter().find_map(|event| {
                matches!(
                    &event.event,
                    maestria_domain::DomainEvent::ApprovalRecorded {
                        approval_id: event_approval_id,
                        outcome,
                    } if *event_approval_id == approval_id && outcome.approved() == approved
                )
                .then_some((event.id, approval_id, approved))
            })
        });
        let validation_report_id = effects.iter().find_map(|effect| {
            let MaestriaEffect::PersistEvent { envelope } = effect else {
                return None;
            };
            let maestria_domain::DomainEvent::ValidationReportCreated { report_id, .. } =
                &envelope.event
            else {
                return None;
            };
            Some(*report_id)
        });
        let durable_event_ids = if durable {
            effects
                .iter()
                .filter_map(|effect| match effect {
                    MaestriaEffect::PersistEvent { envelope } => Some(envelope.id),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        let durable_grant_event_ids = if durable {
            effects
                .iter()
                .filter_map(|effect| {
                    let MaestriaEffect::PersistEvent { envelope } = effect else {
                        return None;
                    };
                    let digest = match &envelope.event {
                        maestria_domain::DomainEvent::RealmReadGrantIssued { grant } => {
                            grant.token_digest().clone()
                        }
                        maestria_domain::DomainEvent::RealmReadGrantRevoked { token_digest } => {
                            token_digest.clone()
                        }
                        _ => return None,
                    };
                    Some((envelope.id, digest))
                })
                .collect()
        } else {
            Vec::new()
        };
        TransitionBarriers {
            proposal_event_id,
            approval,
            validation_report_id,
            durable_event_ids,
            durable_grant_event_ids,
        }
    }

    pub(crate) async fn wait_transition_barriers(
        &self,
        barriers: &TransitionBarriers,
        command_submitted: bool,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        if let Some((event_id, approval_id, approved)) = barriers.approval
            && !self
                .wait_for_approval_resolution(event_id, approval_id, approved, shutdown_token)
                .await
        {
            return false;
        }
        if command_submitted
            && let Some(event_id) = barriers.proposal_event_id
            && !self
                .wait_for_event_persistence(event_id, shutdown_token)
                .await
        {
            return false;
        }
        if command_submitted {
            for event_id in &barriers.durable_event_ids {
                if !self
                    .wait_for_event_persistence(*event_id, shutdown_token)
                    .await
                {
                    return false;
                }
            }
        }
        if command_submitted {
            let expected_grants = {
                let state = self.state.read().await;
                barriers
                    .durable_grant_event_ids
                    .iter()
                    .map(|(event_id, digest)| {
                        state
                            .realm_read_grants
                            .get(digest)
                            .cloned()
                            .map(|grant| (*event_id, grant))
                    })
                    .collect::<Option<Vec<_>>>()
            };
            let Some(expected_grants) = expected_grants else {
                return false;
            };
            for (event_id, grant) in expected_grants {
                if !self
                    .wait_for_realm_read_grant_persistence(event_id, grant, shutdown_token)
                    .await
                {
                    return false;
                }
            }
        }
        true
    }

    pub(crate) async fn finish_validation_barrier(
        &self,
        report_id: Option<maestria_domain::ValidationReportId>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        let Some(report_id) = report_id else {
            return true;
        };
        if self
            .wait_for_validation_report(report_id, shutdown_token)
            .await
        {
            return true;
        }
        if !shutdown_token.is_cancelled() {
            tracing::error!(
                "fatal: timeout or error waiting for durable ValidationReportCreated; stopping runtime"
            );
            shutdown_token.cancel();
        }
        false
    }

    pub(crate) fn reply_domain_error(command: Option<RuntimeCommand>, error: DomainError) {
        if let Some(command) = command {
            let _ = command
                .reply
                .send(Err(RuntimeSubmissionError::DomainRejected {
                    correlation_id: command.correlation_id,
                    error,
                }));
        } else {
            tracing::warn!(%error, "domain rejected input");
        }
    }

    pub(crate) fn reply_admission_error(command: Option<RuntimeCommand>) {
        if let Some(command) = command {
            let _ = command
                .reply
                .send(Err(RuntimeSubmissionError::EffectAdmissionRejected {
                    correlation_id: command.correlation_id,
                }));
        }
    }

    pub(crate) fn reply_preparation_error(command: Option<RuntimeCommand>, reason: String) {
        if let Some(command) = command {
            let _ = command
                .reply
                .send(Err(RuntimeSubmissionError::EffectPreparationRejected {
                    correlation_id: command.correlation_id,
                    reason,
                }));
        }
    }

    pub(crate) fn reply_persistence_error(command: Option<RuntimeCommand>) {
        if let Some(command) = command {
            let _ = command
                .reply
                .send(Err(RuntimeSubmissionError::PersistenceBarrierFailed {
                    correlation_id: command.correlation_id,
                }));
        }
    }
}
