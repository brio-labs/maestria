use crate::config::{Adapters, Governance, RuntimeConfig};
use crate::proposal_recovery::journal_entry_matches_proposal;
use crate::runtime::{DomainApplicationResult, MaestriaRuntime};
use maestria_domain::{
    DomainError, DomainInput, KernelState, MaestriaEffect, ModelAgentProposalExecution,
};
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
                journal_recovery_claims: Arc::new(Mutex::new(BTreeSet::new())),
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
        effect_tx: mpsc::Sender<crate::effect_dispatch::EffectBatch>,
    ) {
        let adapters = Arc::clone(&self.adapters);
        let scope_id = self.config.scope_id;
        tokio::spawn(async move {
            let mut proposals = BTreeMap::new();
            let mut approval_owned_runs = BTreeSet::new();
            match adapters.effect_journal.scan_in_flight() {
                Ok(entries) => {
                    for entry in entries {
                        if entry.status != maestria_ports::EffectJournalStatus::FeedbackAccepted
                            || entry.feedback.is_none()
                            || snapshot.model_agent_results.contains_key(&entry.run_id)
                        {
                            continue;
                        }
                        let Some(proposal) = snapshot.model_agent_requests.get(&entry.run_id)
                        else {
                            continue;
                        };
                        if !matches!(&proposal.execution, ModelAgentProposalExecution::Fresh) {
                            continue;
                        }
                        if !journal_entry_matches_proposal(&entry, proposal, scope_id) {
                            continue;
                        }
                        let mut resumed = proposal.clone();
                        resumed.execution = ModelAgentProposalExecution::JournalRecovery {
                            journal_generation: entry.generation,
                        };
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
                if matches!(&proposal.execution, ModelAgentProposalExecution::Fresh)
                    && !snapshot.model_agent_results.contains_key(&proposal.run_id)
                    && !approval_owned_runs.contains(&proposal.run_id)
                    && !proposals.contains_key(&proposal.run_id)
                {
                    proposals.insert(proposal.run_id, proposal.clone());
                }
            }
            for proposal in proposals.into_values() {
                tokio::select! {
                    () = shutdown_token.cancelled() => break,
                    result = effect_tx.send(vec![MaestriaEffect::QueryHarnessProposal(
                        proposal.into_harness_request(),
                    )]) => {
                        if let Err(error) = result {
                            tracing::warn!(%error, "model-agent recovery effect channel closed");
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
        input_rx: mpsc::Receiver<DomainInput>,
        shutdown_token: tokio_util::sync::CancellationToken,
    ) {
        let (effect_tx, effect_rx) =
            mpsc::channel::<crate::effect_dispatch::EffectBatch>(self.config.input_buffer_size);
        let effect_shutdown = tokio_util::sync::CancellationToken::new();
        let effect_executor =
            self.spawn_effect_executor(effect_rx, effect_shutdown.clone(), shutdown_token.clone());
        let recovery_snapshot = self.state.read().await.clone();
        self.spawn_model_agent_recovery(
            shutdown_token.clone(),
            recovery_snapshot,
            effect_tx.clone(),
        );
        let Some(command_rx) = self.command_rx.take() else {
            tracing::error!("runtime command receiver missing");
            return;
        };

        self.run_input_loop(input_rx, command_rx, &effect_tx, &shutdown_token)
            .await;
        drop(effect_tx);
        if !self.config.drain_effects_on_shutdown {
            effect_shutdown.cancel();
        }
        shutdown_token.cancel();
        if let Err(error) = effect_executor.await {
            tracing::error!(%error, "effect executor task failed");
        }
    }

    async fn run_input_loop(
        &self,
        mut input_rx: mpsc::Receiver<DomainInput>,
        mut command_rx: mpsc::Receiver<crate::runtime::RuntimeCommand>,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) {
        loop {
            let incoming = tokio::select! {
                () = shutdown_token.cancelled() => break,
                input = input_rx.recv() => input.map(|input| (input, None)),
                command = command_rx.recv() => {
                    command.map(|command| (command.input.clone(), Some(command)))
                },
            };
            let Some((input, command)) = incoming else {
                break;
            };
            if !self
                .process_input(input, command, effect_tx, shutdown_token)
                .await
            {
                break;
            }
        }
    }

    async fn process_input(
        &self,
        input: DomainInput,
        mut command: Option<crate::runtime::RuntimeCommand>,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        let input = Self::correlate_proposal(input, command.as_ref());
        let approval_continuation = self.approval_continuation(&input);
        if let Some(detail) = self.boundary_error(&input).await {
            Self::reply_domain_error(command, DomainError::InternalInvariantViolation { detail });
            return true;
        }
        let harness_feedback = Self::harness_feedback(&input);
        let approval_barrier = Self::approval_barrier(&input, command.as_ref());
        let (candidate, output, should_resume_approval) = match self
            .stage_input(input, approval_continuation.is_some())
            .await
        {
            Ok(staged) => staged,
            Err(error) => {
                Self::reply_domain_error(command, error);
                return true;
            }
        };
        let permit = if output.effects.is_empty() {
            None
        } else {
            match self.reserve_effect_batch(effect_tx, shutdown_token).await {
                Ok(permit) => Some(permit),
                Err(_) => {
                    Self::reply_admission_error(command);
                    return !shutdown_token.is_cancelled();
                }
            }
        };
        let effects_admitted = output.effects.len();
        let events = output.events.clone();
        let barriers = Self::transition_barriers(&events, &output.effects, approval_barrier);
        {
            let mut state = self.state.write().await;
            *state = candidate;
            self.register_harness_feedback(harness_feedback, &output.effects);
        }
        if let Some(permit) = permit
            && self.send_reserved_effects(permit, output.effects).is_err()
        {
            Self::reply_admission_error(command);
            shutdown_token.cancel();
            return false;
        }
        if !self
            .wait_transition_barriers(&barriers, command.is_some(), shutdown_token)
            .await
        {
            Self::reply_persistence_error(command);
            return true;
        }
        if should_resume_approval
            && let Some(proposal) = approval_continuation
            && matches!(
                proposal.execution,
                ModelAgentProposalExecution::ApprovalContinuation { .. }
            )
        {
            self.resume_model_agent_after_approval(proposal, shutdown_token.clone());
        }
        if let Some(command) = command.take() {
            let _ = command.reply.send(Ok(DomainApplicationResult {
                correlation_id: command.correlation_id,
                events,
                effects_admitted,
            }));
        }
        self.finish_validation_barrier(barriers.validation_report_id, shutdown_token)
            .await
    }
}
