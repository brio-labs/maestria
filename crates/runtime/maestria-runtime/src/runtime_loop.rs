use crate::config::{Adapters, Governance, RuntimeConfig};
use crate::runtime::{DomainApplicationResult, MaestriaRuntime, RuntimeSubmissionError};
use maestria_domain::{DomainError, DomainInput, KernelState, MaestriaEffect};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, atomic::AtomicU64};
use tokio::sync::{RwLock, mpsc};

impl MaestriaRuntime {
    pub fn new(
        mut config: RuntimeConfig,
        state: KernelState,
        adapters: Adapters,
        governance: Governance,
    ) -> (Self, mpsc::Receiver<DomainInput>) {
        config.max_concurrent_effects = config.max_concurrent_effects.max(1);
        let (input_tx, input_rx) = mpsc::channel(config.input_buffer_size);
        let (command_tx, command_rx) = mpsc::channel(config.input_buffer_size);
        let next_command_id = Arc::new(AtomicU64::new(1));
        let next_validation_report_id =
            Arc::new(AtomicU64::new(Self::seed_next_validation_report_id(&state)));
        (
            Self {
                config,
                state: Arc::new(RwLock::new(state)),
                adapters: Arc::new(adapters),
                governance: Arc::new(governance),
                input_tx,
                command_tx,
                command_rx: Some(command_rx),
                next_command_id,
                feedback_acks: Arc::new(Mutex::new(BTreeMap::new())),
                next_validation_report_id,
                #[cfg(test)]
                test_pre_failed_effect_task: false,
            },
            input_rx,
        )
    }

    fn spawn_model_agent_recovery(
        &self,
        shutdown_token: tokio_util::sync::CancellationToken,
        snapshot: KernelState,
    ) {
        let adapters = Arc::clone(&self.adapters);
        let input_tx = self.input_tx.clone();
        tokio::spawn(async move {
            let mut proposals = BTreeMap::new();
            let mut approval_owned_runs = BTreeSet::new();
            match adapters.effect_journal.scan_in_flight() {
                Ok(entries) => {
                    for entry in entries {
                        if entry.feedback.is_none()
                            || snapshot.model_agent_results.contains_key(&entry.run_id)
                        {
                            continue;
                        }
                        let Some(proposal) = snapshot.model_agent_requests.get(&entry.run_id)
                        else {
                            continue;
                        };
                        let mut resumed = proposal.clone();
                        resumed.journal_generation = Some(entry.generation);
                        proposals.insert(entry.run_id, resumed);
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "failed to scan effect journal for model-agent recovery");
                }
            }
            match adapters.approval_repo.find_all() {
                Ok(records) => {
                    for record in records {
                        let Some(proposal) =
                            crate::effect_execution::decode_pending_continuation(&record)
                        else {
                            continue;
                        };
                        if !snapshot.model_agent_requests.contains_key(&proposal.run_id) {
                            continue;
                        }
                        approval_owned_runs.insert(proposal.run_id);
                        if !matches!(
                            record.status,
                            maestria_ports::ApprovalStatus::Approved
                                | maestria_ports::ApprovalStatus::Denied
                        ) || snapshot.model_agent_results.contains_key(&proposal.run_id)
                        {
                            continue;
                        }
                        proposals.entry(proposal.run_id).or_insert(proposal);
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "failed to scan approvals for model-agent recovery");
                }
            }
            for proposal in snapshot.model_agent_requests.values() {
                if !snapshot.model_agent_results.contains_key(&proposal.run_id)
                    && !approval_owned_runs.contains(&proposal.run_id)
                    && !proposals.contains_key(&proposal.run_id)
                {
                    proposals.insert(proposal.run_id, proposal.clone());
                }
            }
            for proposal in proposals.into_values() {
                tokio::select! {
                    () = shutdown_token.cancelled() => break,
                    result = input_tx.send(DomainInput::ModelAgentProposalResumed(proposal)) => {
                        if let Err(error) = result {
                            tracing::warn!(%error, "model-agent recovery input channel closed");
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Runs the domain-input loop until the shutdown token is cancelled or
    /// the input channel closes.
    ///
    /// Cancellation stops accepting new inputs. By default, in-flight effects
    /// are cancelled; call [`Self::with_graceful_shutdown`] before `run` to
    /// drain already-started effects. The method returns after the effect
    /// executor has observed the selected shutdown policy.
    pub async fn run(
        mut self,
        mut input_rx: mpsc::Receiver<DomainInput>,
        shutdown_token: tokio_util::sync::CancellationToken,
    ) {
        let (effect_tx, effect_rx) =
            mpsc::channel::<crate::effect_dispatch::EffectBatch>(self.config.input_buffer_size);
        let effect_shutdown = tokio_util::sync::CancellationToken::new();
        let effect_executor =
            self.spawn_effect_executor(effect_rx, effect_shutdown.clone(), shutdown_token.clone());

        let recovery_snapshot = self.state.read().await.clone();
        self.spawn_model_agent_recovery(shutdown_token.clone(), recovery_snapshot);
        let Some(mut command_rx) = self.command_rx.take() else {
            tracing::error!("runtime command receiver missing");
            return;
        };
        loop {
            let incoming = tokio::select! {
                () = shutdown_token.cancelled() => break,
                msg = input_rx.recv() => msg.map(|input| (input, None)),
                msg = command_rx.recv() => msg.map(|command| (command.input.clone(), Some(command))),
            };
            let Some((input, command)) = incoming else {
                break;
            };
            let input = match input {
                DomainInput::ModelAgentProposalRequested(mut proposal) => {
                    if let Some(command) = &command {
                        proposal.correlation_id = command.correlation_id;
                    }
                    DomainInput::ModelAgentProposalRequested(proposal)
                }
                other => other,
            };
            let approval_continuation = match &input {
                DomainInput::ApprovalResolved(decision) => self
                    .adapters
                    .approval_repo
                    .find_by_id(decision.approval_id)
                    .ok()
                    .flatten()
                    .and_then(|record| {
                        crate::effect_execution::decode_pending_continuation(&record)
                    }),
                _ => None,
            };
            let boundary_error = match &input {
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
            };
            if let Some(detail) = boundary_error {
                if let Some(command) = command {
                    let _ = command
                        .reply
                        .send(Err(RuntimeSubmissionError::DomainRejected {
                            correlation_id: command.correlation_id,
                            error: DomainError::InternalInvariantViolation { detail },
                        }));
                }
                continue;
            }

            let harness_feedback = match &input {
                DomainInput::HarnessRunCompleted(completion) => {
                    Some((completion.run_id, completion.generation))
                }
                _ => None,
            };
            let approval_barrier = match (&input, command.as_ref()) {
                (DomainInput::ApprovalResolved(decision), Some(_)) => {
                    Some((decision.approval_id, decision.approved))
                }
                _ => None,
            };

            // The runtime loop is the sole state writer. Stage from a read
            // snapshot, then release it before waiting for effect capacity so
            // in-flight persistence effects can continue reading state.
            let state = self.state.read().await;
            let mut candidate = state.clone();
            let should_resume_approval = matches!(
                &input,
                DomainInput::ApprovalResolved(decision)
                    if approval_continuation.is_some()
                        && !state.resolved_approvals.contains(&decision.approval_id)
            );
            drop(state);
            let output = match candidate.apply_input(input) {
                Ok(output) => output,
                Err(error) => {
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
                    continue;
                }
            };
            let permit = if output.effects.is_empty() {
                None
            } else {
                match self.reserve_effect_batch(&effect_tx, &shutdown_token).await {
                    Ok(permit) => Some(permit),
                    Err(_) => {
                        if let Some(command) = command {
                            let _ = command.reply.send(Err(
                                RuntimeSubmissionError::EffectAdmissionRejected {
                                    correlation_id: command.correlation_id,
                                },
                            ));
                        }
                        if shutdown_token.is_cancelled() {
                            break;
                        }
                        continue;
                    }
                }
            };
            let effects_admitted = output.effects.len();
            let events = output.events.clone();
            let mut state = self.state.write().await;
            let proposal_event_id = events.iter().find_map(|event| {
                matches!(
                    &event.event,
                    maestria_domain::DomainEvent::ModelAgentProposalRequested { .. }
                )
                .then_some(event.id)
            });
            let approval_event_barrier = approval_barrier.and_then(|(approval_id, approved)| {
                events.iter().find_map(|event| {
                    matches!(
                        &event.event,
                        maestria_domain::DomainEvent::ApprovalRecorded {
                            approval_id: event_approval_id,
                            approved: event_approved,
                            ..
                        } if *event_approval_id == approval_id && *event_approved == approved
                    )
                    .then_some((event.id, approval_id, approved))
                })
            });
            *state = candidate;
            let mut wait_for_report_id = None;
            for effect in &output.effects {
                if let MaestriaEffect::PersistEvent { envelope } = effect
                    && let maestria_domain::DomainEvent::ValidationReportCreated {
                        report_id, ..
                    } = &envelope.event
                {
                    wait_for_report_id = Some(*report_id);
                }
            }
            self.register_harness_feedback(harness_feedback, &output.effects);
            drop(state);
            if let Some(permit) = permit
                && self.send_reserved_effects(permit, output.effects).is_err()
            {
                if let Some(command) = command {
                    let _ =
                        command
                            .reply
                            .send(Err(RuntimeSubmissionError::EffectAdmissionRejected {
                                correlation_id: command.correlation_id,
                            }));
                }
                shutdown_token.cancel();
                break;
            }
            let approval_barrier_failed = match approval_barrier {
                Some((approval_id, approved)) => match approval_event_barrier {
                    Some((event_id, event_approval_id, event_approved)) => {
                        debug_assert_eq!(approval_id, event_approval_id);
                        debug_assert_eq!(approved, event_approved);
                        !self
                            .wait_for_approval_resolution(
                                event_id,
                                event_approval_id,
                                event_approved,
                                &shutdown_token,
                            )
                            .await
                    }
                    None => true,
                },
                None => false,
            };
            let proposal_barrier_failed = if let (Some(event_id), Some(_command_ref)) =
                (proposal_event_id, command.as_ref())
            {
                !self
                    .wait_for_event_persistence(event_id, &shutdown_token)
                    .await
            } else {
                false
            };
            if approval_barrier_failed || proposal_barrier_failed {
                if let Some(command) = command {
                    let correlation_id = command.correlation_id;
                    let _ =
                        command
                            .reply
                            .send(Err(RuntimeSubmissionError::PersistenceBarrierFailed {
                                correlation_id,
                            }));
                }
                continue;
            }
            if should_resume_approval
                && let Some(proposal) = approval_continuation
                && proposal.approval_id.is_some()
            {
                self.resume_model_agent_after_approval(proposal, shutdown_token.clone());
            }
            if let Some(command) = command {
                let _ = command.reply.send(Ok(DomainApplicationResult {
                    correlation_id: command.correlation_id,
                    events,
                    effects_admitted,
                }));
            }
            if let Some(report_id) = wait_for_report_id {
                let found = self
                    .wait_for_validation_report(report_id, &shutdown_token)
                    .await;
                if !found {
                    if !shutdown_token.is_cancelled() {
                        tracing::error!(
                            "fatal: timeout or error waiting for durable ValidationReportCreated; stopping runtime"
                        );
                        shutdown_token.cancel();
                    }
                    break;
                }
            }
        }

        drop(effect_tx);
        if !self.config.drain_effects_on_shutdown {
            effect_shutdown.cancel();
        }
        shutdown_token.cancel();
        if let Err(error) = effect_executor.await {
            tracing::error!(%error, "effect executor task failed");
        }
    }
}
