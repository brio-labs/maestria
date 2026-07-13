use crate::config::EffectExecutionContext;
use crate::shell_policy::{cat_path_args, is_shell_grammar_allowed, resolve_working_directory};
use maestria_domain::{DomainInput, HarnessRunCompleted, QueryHarnessRequest};
use maestria_ports::{HarnessCommandClass, HarnessRequest};
use std::path::PathBuf;

impl EffectExecutionContext {
    /// Execute a harness command on behalf of a task.
    /// Applies shell grammar restrictions and scope containment before
    /// delegating to the harness adapter. Sends HarnessRunCompleted
    /// back to the domain loop.
    pub(crate) async fn handle_query_harness(&self, request: QueryHarnessRequest) -> bool {
        let (class, working_directory) = match self.gate_harness_request(&request) {
            Ok(res) => res,
            Err(success) => return success,
        };

        let intent = maestria_ports::EffectJournalIntent {
            run_id: request.run_id,
            task_id: request.task_id,
            capability: request.capability.clone(),
            command: request.command.clone(),
            scope_id: self.scope_id,
            requested_generation: request.generation,
        };

        let entry = match self.adapters.effect_journal.record_intent(intent) {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(%e, "failed to record harness intent");
                return false;
            }
        };

        if let Err(e) = self
            .adapters
            .effect_journal
            .record_started(request.run_id, entry.generation)
        {
            tracing::error!(%e, "failed to record harness start");
            return false;
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
    }

    fn gate_harness_request(
        &self,
        request: &QueryHarnessRequest,
    ) -> Result<(HarnessCommandClass, PathBuf), bool> {
        let class = match request.capability.as_str() {
            "browser" => HarnessCommandClass::Browser,
            "fetch" | "web" => HarnessCommandClass::Fetch,
            "shell" => HarnessCommandClass::Shell,
            other => {
                tracing::error!(capability = other, "Unknown harness capability requested");
                return Err(true);
            }
        };

        // ── scope capability gate ────────────────────────────────
        let scope_guard = maestria_governance::ScopeGuard::new(self.scope.clone());
        let scope = scope_guard.scope();
        if !scope.harness_allowed(&request.capability) {
            tracing::warn!(capability = %request.capability, "Scope does not allow this harness; not spawning");
            return Err(true);
        }
        if !scope.command_allowed(&request.command) {
            tracing::warn!(command = %request.command, "command blocked by scope; not spawning");
            return Err(true);
        }
        if !is_shell_grammar_allowed(&request.command) {
            tracing::warn!(
                command = %request.command,
                "command violates shell grammar restrictions; not spawning"
            );
            return Err(true);
        }

        // ── cat path containment ─────────────────────────────────
        if class == HarnessCommandClass::Shell && request.command.trim().starts_with("cat") {
            for arg in cat_path_args(&request.command) {
                let path = PathBuf::from(arg);
                if let Err(containment_err) = scope_guard.check_read_containment(&path) {
                    tracing::warn!(
                        path = %path.display(),
                        ?containment_err,
                        "cat path outside readable roots; not spawning"
                    );
                    return Err(true);
                }
            }
        }
        let working_directory = match resolve_working_directory(scope) {
            Ok(path) => path,
            Err(error) => {
                tracing::error!(%error, "unable to resolve harness working directory");
                return Err(false);
            }
        };

        Ok((class, working_directory))
    }

    async fn execute_and_process_harness(
        &self,
        request: QueryHarnessRequest,
        harness_request: HarnessRequest,
        generation: u64,
    ) -> bool {
        match self.adapters.harness.execute(harness_request).await {
            Ok(outcome) => {
                // Claim the generation atomically before enqueueing feedback.
                // A newer intent supersedes this claim and is rejected by the
                // runtime boundary before the domain can observe stale output.
                if let Err(error) = self
                    .adapters
                    .effect_journal
                    .claim_feedback(request.run_id, generation)
                {
                    tracing::warn!(
                        run_id = %request.run_id,
                        %generation,
                        %error,
                        "harness feedback rejected as stale"
                    );
                    return true;
                }

                let mut output = String::from_utf8_lossy(&outcome.stdout).into_owned();
                if !outcome.stderr.is_empty() {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&String::from_utf8_lossy(&outcome.stderr));
                }

                let domain_input = DomainInput::HarnessRunCompleted(HarnessRunCompleted {
                    run_id: request.run_id,
                    generation,
                    task_id: request.task_id,
                    command: outcome.command,
                    exit_code: outcome.exit_code,
                    output,
                });

                if let Err(error) =
                    Self::send_input(&self.input_tx, domain_input, "harness completion")
                {
                    tracing::error!(%error, "failed to deliver harness completion; pausing effect");
                    if let Err(journal_error) = self.adapters.effect_journal.record_terminal(
                        request.run_id,
                        generation,
                        maestria_ports::EffectJournalStatus::Paused,
                    ) {
                        tracing::error!(%journal_error, "failed to pause saturated harness effect");
                    }
                }

                true
            }
            Err(error) => {
                let _ = self.adapters.effect_journal.record_terminal(
                    request.run_id,
                    generation,
                    maestria_ports::EffectJournalStatus::Failed,
                );
                tracing::error!(run_id = %request.run_id, %error, "harness execution failed");
                false
            }
        }
    }
}
