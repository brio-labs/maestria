use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use maestria_domain::{DomainInput, HarnessRunCompleted, HarnessRunId, QueryHarnessRequest};
use maestria_ports::{EffectJournalStatus, HarnessOutcome, HarnessRequest, PortError};

impl EffectExecutionContext {
    /// Execute a harness command on behalf of a task.
    /// Applies shell grammar restrictions and scope containment before
    /// delegating to the harness adapter. Sends HarnessRunCompleted
    /// back to the domain loop.
    pub(crate) async fn handle_query_harness(
        &self,
        request: QueryHarnessRequest,
    ) -> Result<(), EffectFailure> {
        let (class, working_directory) = self.gate_harness_request(&request)?;

        let intent = maestria_ports::EffectJournalIntent {
            run_id: request.run_id,
            task_id: request.task_id,
            capability: request.capability.clone(),
            command: request.command.clone(),
            scope_id: self.scope_id,
            requested_generation: request.generation,
        };

        let entry = match self.adapters.effect_journal.record_intent(intent) {
            Ok(entry) => entry,
            Err(error) => {
                tracing::error!(%error, "failed to record harness intent");
                return Err(EffectFailure::Failed(format!(
                    "failed to record harness intent: {error}"
                )));
            }
        };

        if let Err(error) = self
            .adapters
            .effect_journal
            .record_started(request.run_id, entry.generation)
        {
            tracing::error!(%error, "failed to record harness start");
            return Err(EffectFailure::Failed(format!(
                "failed to record harness start: {error}"
            )));
        }

        let scope_guard = maestria_governance::ScopeGuard::new(self.scope.clone());
        let scope = scope_guard.scope();
        let harness_request = HarnessRequest {
            run_id: request.run_id,
            command: request.command.clone(),
            working_directory,
            duration_budget: self.default_effect_timeout,
            class,
            readable_roots: scope.readable_roots().to_vec(),
            blocked_paths: scope.blocked_paths().to_vec(),
            blocked_patterns: scope.blocked_patterns().to_vec(),
        };

        self.execute_and_process_harness(request, harness_request, entry.generation)
            .await
            .map(|_| ())
    }

    pub(crate) async fn execute_and_process_harness(
        &self,
        request: QueryHarnessRequest,
        harness_request: HarnessRequest,
        generation: u64,
    ) -> Result<Option<HarnessOutcome>, EffectFailure> {
        let (outcome, was_stored) = self
            .execute_harness_provider(&request, harness_request, generation)
            .await?;
        if !self.claim_harness_feedback(&request, generation, &outcome, was_stored)? {
            return Ok(None);
        }
        self.deliver_harness_completion(&request, &outcome, generation, was_stored)?;
        Ok(Some(outcome))
    }

    /// Read the durably accepted outcome for a harness run, or execute the
    /// harness provider when none exists yet. Returns the outcome together
    /// with whether it came from the journal.
    async fn execute_harness_provider(
        &self,
        request: &QueryHarnessRequest,
        harness_request: HarnessRequest,
        generation: u64,
    ) -> Result<(HarnessOutcome, bool), EffectFailure> {
        let stored_outcome = self
            .adapters
            .effect_journal
            .feedback_outcome(request.run_id, generation)
            .map_err(|error| {
                EffectFailure::Failed(format!("read stored harness feedback: {error}"))
            })?;
        let outcome = if let Some(outcome) = stored_outcome.clone() {
            outcome
        } else {
            self.adapters
                .harness
                .execute(harness_request)
                .await
                .map_err(|error| {
                    let terminal = self.record_harness_terminal(
                        request.run_id,
                        generation,
                        EffectJournalStatus::Failed,
                    );
                    match terminal {
                        Ok(()) => EffectFailure::Failed(format!("harness execution failed: {error}")),
                        Err(journal_error) => EffectFailure::Failed(format!(
                            "harness execution failed: {error}; additionally failed to record terminal journal state: {journal_error}"
                        )),
                    }
                })?
        };
        Ok((outcome, stored_outcome.is_some()))
    }

    /// Claim the fresh harness outcome in the journal. Returns `Ok(true)`
    /// when the outcome was accepted (or was already stored by an earlier
    /// process), and `Ok(false)` when the claim was rejected as stale — the
    /// caller must stop processing the run without delivering feedback.
    fn claim_harness_feedback(
        &self,
        request: &QueryHarnessRequest,
        generation: u64,
        outcome: &HarnessOutcome,
        was_stored: bool,
    ) -> Result<bool, EffectFailure> {
        if was_stored {
            // The outcome was atomically accepted by an earlier process.
            return Ok(true);
        }
        match self.adapters.effect_journal.claim_feedback_with_outcome(
            request.run_id,
            generation,
            outcome.clone(),
        ) {
            Ok(()) => Ok(true),
            Err(error) if error.is_not_found() => {
                tracing::warn!(
                    run_id = %request.run_id,
                    %generation,
                    %error,
                    "harness feedback rejected as stale"
                );
                Ok(false)
            }
            Err(error) => Err(EffectFailure::Failed(format!(
                "harness feedback claim failed: {error}"
            ))),
        }
    }

    /// Deliver the harness completion input to the domain loop, pausing or
    /// degrading the effect when delivery fails.
    fn deliver_harness_completion(
        &self,
        request: &QueryHarnessRequest,
        outcome: &HarnessOutcome,
        generation: u64,
        was_stored: bool,
    ) -> Result<(), EffectFailure> {
        let delivery = Self::send_input(
            &self.input_tx,
            harness_completion_input(request.run_id, generation, request.task_id, outcome),
            "harness completion",
        );
        if let Err(error) = delivery {
            if was_stored {
                return Err(EffectFailure::Degraded(format!(
                    "harness completion delivery failed: {error}; effect remains recoverable"
                )));
            }
            return match self.record_harness_terminal(
                request.run_id,
                generation,
                EffectJournalStatus::Paused,
            ) {
                Ok(()) => Err(EffectFailure::Degraded(format!(
                    "harness completion delivery failed: {error}; effect paused"
                ))),
                Err(journal_error) => Err(EffectFailure::Failed(format!(
                    "harness completion delivery failed: {error}; failed to pause harness effect: {journal_error}"
                ))),
            };
        }
        Ok(())
    }

    /// Record a terminal journal state for a harness run.
    fn record_harness_terminal(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError> {
        self.adapters
            .effect_journal
            .record_terminal(run_id, generation, status)
    }
}

/// Build the domain completion input for a finished harness run.
///
/// One owner of the completion-output contract: stdout and stderr are merged
/// with a newline separator so live and recovered deliveries cannot drift
/// (R28).
pub(crate) fn harness_completion_input(
    run_id: maestria_domain::HarnessRunId,
    generation: u64,
    task_id: Option<maestria_domain::TaskId>,
    outcome: &maestria_ports::HarnessOutcome,
) -> DomainInput {
    let mut output = String::from_utf8_lossy(&outcome.stdout).into_owned();
    if !outcome.stderr.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&String::from_utf8_lossy(&outcome.stderr));
    }
    DomainInput::HarnessRunCompleted(HarnessRunCompleted {
        run_id,
        generation,
        task_id,
        command: outcome.command.clone(),
        exit_code: outcome.exit_code,
        output,
    })
}

pub(crate) fn truncate_output(bytes: &[u8]) -> String {
    const LIMIT: usize = 4096;
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= LIMIT {
        return text.into_owned();
    }
    let mut end = LIMIT - 3;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}
