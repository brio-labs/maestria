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
        let adapters = &self.adapters;

        let class = match request.capability.as_str() {
            "browser" => HarnessCommandClass::Browser,
            "fetch" | "web" => HarnessCommandClass::Fetch,
            "shell" => HarnessCommandClass::Shell,
            other => {
                tracing::error!(capability = other, "Unknown harness capability requested");
                return true;
            }
        };

        // ── grammar restriction ──────────────────────────────────
        if !is_shell_grammar_allowed(&request.command) {
            tracing::warn!(
                command = %request.command,
                "command violates shell grammar restrictions; not spawning"
            );
            return true;
        }

        // ── cat path containment ─────────────────────────────────
        let scope = maestria_governance::ScopeGuard::new(self.scope.clone());
        if class == HarnessCommandClass::Shell && request.command.trim().starts_with("cat") {
            for arg in cat_path_args(&request.command) {
                let path = PathBuf::from(arg);
                if let Err(containment_err) = scope.check_read_containment(&path) {
                    tracing::warn!(
                        path = %path.display(),
                        ?containment_err,
                        "cat path outside readable roots; not spawning"
                    );
                    return true;
                }
            }
        }

        let working_directory = match resolve_working_directory(scope.scope()) {
            Ok(path) => path,
            Err(error) => {
                tracing::error!(%error, "unable to resolve harness working directory");
                return false;
            }
        };
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

        match adapters.harness.execute(harness_request).await {
            Ok(outcome) => {
                let mut output = String::from_utf8_lossy(&outcome.stdout).into_owned();
                if !outcome.stderr.is_empty() {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&String::from_utf8_lossy(&outcome.stderr));
                }
                Self::send_input(
                    &self.input_tx,
                    DomainInput::HarnessRunCompleted(HarnessRunCompleted {
                        task_id: request.task_id,
                        command: outcome.command,
                        exit_code: outcome.exit_code,
                        output,
                    }),
                    "harness completion",
                )
                .await;
                true
            }
            Err(error) => {
                tracing::error!(run_id = %request.run_id, %error, "harness execution failed");
                false
            }
        }
    }
}
