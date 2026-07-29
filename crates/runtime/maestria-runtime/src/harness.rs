use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use crate::shell_policy::{cat_path_args, is_shell_grammar_allowed, resolve_working_directory};
use maestria_domain::{DomainInput, HarnessRunCompleted, QueryHarnessRequest};
use maestria_ports::{HarnessCommandClass, HarnessRequest};
use std::path::{Path, PathBuf};

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

    pub(crate) fn gate_harness_request(
        &self,
        request: &QueryHarnessRequest,
    ) -> Result<(HarnessCommandClass, PathBuf), EffectFailure> {
        let class = match request.capability.as_str() {
            "browser" => HarnessCommandClass::Browser,
            "fetch" | "web" => HarnessCommandClass::Fetch,
            "shell" => HarnessCommandClass::Shell,
            other => {
                tracing::error!(capability = other, "Unknown harness capability requested");
                return Err(EffectFailure::Denied(format!(
                    "unsupported harness capability: {other}"
                )));
            }
        };

        // ── scope capability gate ────────────────────────────────
        let scope_guard = maestria_governance::ScopeGuard::new(self.scope.clone());
        let scope = scope_guard.scope();
        if !scope.harness_allowed(&request.capability) {
            tracing::warn!(capability = %request.capability, "Scope does not allow this harness; not spawning");
            return Err(EffectFailure::Denied(format!(
                "harness capability `{}` is not allowed by scope",
                request.capability
            )));
        }
        if !scope.command_allowed(&request.command) {
            tracing::warn!(command = %request.command, "command blocked by scope; not spawning");
            return Err(EffectFailure::Denied(format!(
                "command `{}` is blocked by scope",
                request.command
            )));
        }
        if !is_shell_grammar_allowed(&request.command) {
            tracing::warn!(
                command = %request.command,
                "command violates shell grammar restrictions; not spawning"
            );
            return Err(EffectFailure::Denied(format!(
                "command `{}` violates shell grammar restrictions",
                request.command
            )));
        }

        // ── harness working directory ─────────────────────────────
        let working_directory = match resolve_working_directory(scope) {
            Ok(path) => path,
            Err(error) => {
                tracing::error!(%error, "unable to resolve harness working directory");
                return Err(EffectFailure::Failed(format!(
                    "unable to resolve harness working directory: {error}"
                )));
            }
        };

        // ── cat path policy ────────────────────────────────────────
        if class == HarnessCommandClass::Shell && request.command.trim().starts_with("cat") {
            for arg in cat_path_args(&request.command) {
                let path = resolve_cat_path(arg, &working_directory);
                if let Err(containment_err) = scope_guard.check_read_containment(&path) {
                    tracing::warn!(
                        path = %path.display(),
                        ?containment_err,
                        "cat path outside readable roots; not spawning"
                    );
                    return Err(EffectFailure::Denied(format!(
                        "cat path `{}` is outside readable scope ({containment_err:?})",
                        path.display()
                    )));
                }
                if path_is_blocked(&path, &working_directory, scope.blocked_paths()) {
                    tracing::warn!(
                        path = %path.display(),
                        "cat path blocked by scope; not spawning"
                    );
                    return Err(EffectFailure::Denied(format!(
                        "cat path `{}` is blocked by scope",
                        path.display()
                    )));
                }
                if path_matches_blocked_pattern(&path, scope.blocked_patterns()) {
                    tracing::warn!(
                        path = %path.display(),
                        "cat path matches a blocked scope pattern; not spawning"
                    );
                    return Err(EffectFailure::Denied(format!(
                        "cat path `{}` matches a blocked scope pattern",
                        path.display()
                    )));
                }
            }
        }

        Ok((class, working_directory))
    }

    pub(crate) async fn execute_and_process_harness(
        &self,
        request: QueryHarnessRequest,
        harness_request: HarnessRequest,
        generation: u64,
    ) -> Result<Option<maestria_ports::HarnessOutcome>, EffectFailure> {
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
                    let _ = self.adapters.effect_journal.record_terminal(
                        request.run_id,
                        generation,
                        maestria_ports::EffectJournalStatus::Failed,
                    );
                    EffectFailure::Failed(format!("harness execution failed: {error}"))
                })?
        };
        if stored_outcome.is_some() {
            // The outcome was atomically accepted by an earlier process.
        } else if let Err(error) = self.adapters.effect_journal.claim_feedback_with_outcome(
            request.run_id,
            generation,
            outcome.clone(),
        ) {
            if error.is_not_found() {
                tracing::warn!(
                    run_id = %request.run_id,
                    %generation,
                    %error,
                    "harness feedback rejected as stale"
                );
                return Ok(None);
            }
            return Err(EffectFailure::Failed(format!(
                "harness feedback claim failed: {error}"
            )));
        }
        let mut output = String::from_utf8_lossy(&outcome.stdout).into_owned();
        if !outcome.stderr.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&String::from_utf8_lossy(&outcome.stderr));
        }
        let delivery = Self::send_input(
            &self.input_tx,
            DomainInput::HarnessRunCompleted(HarnessRunCompleted {
                run_id: request.run_id,
                generation,
                task_id: request.task_id,
                command: outcome.command.clone(),
                exit_code: outcome.exit_code,
                output,
            }),
            "harness completion",
        );
        if let Err(error) = delivery {
            if stored_outcome.is_some() {
                return Err(EffectFailure::Degraded(format!(
                    "harness completion delivery failed: {error}; effect remains recoverable"
                )));
            }
            return match self.adapters.effect_journal.record_terminal(
                request.run_id,
                generation,
                maestria_ports::EffectJournalStatus::Paused,
            ) {
                Ok(()) => Err(EffectFailure::Degraded(format!(
                    "harness completion delivery failed: {error}; effect paused"
                ))),
                Err(journal_error) => Err(EffectFailure::Failed(format!(
                    "harness completion delivery failed: {error}; failed to pause harness effect: {journal_error}"
                ))),
            };
        }
        Ok(Some(outcome))
    }
}

fn resolve_cat_path(raw_path: &str, working_directory: &Path) -> PathBuf {
    let path = Path::new(raw_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_directory.join(path)
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn path_is_blocked(path: &Path, working_directory: &Path, blocked_paths: &[PathBuf]) -> bool {
    let normalized = normalize_path(path);
    blocked_paths.iter().any(|blocked| {
        let blocked = if blocked.is_absolute() {
            blocked.clone()
        } else {
            working_directory.join(blocked)
        };
        normalized.starts_with(normalize_path(&blocked))
    })
}

fn path_matches_blocked_pattern(path: &Path, patterns: &[String]) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        patterns
            .iter()
            .any(|pattern| filename_matches(&name, pattern))
    })
}

fn filename_matches(name: &str, pattern: &str) -> bool {
    if !pattern.contains('*') && !pattern.contains('?') {
        return name == pattern;
    }
    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let mut matches = vec![vec![false; pattern.len() + 1]; name.len() + 1];
    matches[0][0] = true;
    for pattern_index in 1..=pattern.len() {
        if pattern[pattern_index - 1] == '*' {
            matches[0][pattern_index] = matches[0][pattern_index - 1];
        }
    }
    for name_index in 1..=name.len() {
        for pattern_index in 1..=pattern.len() {
            matches[name_index][pattern_index] = match pattern[pattern_index - 1] {
                '*' => {
                    matches[name_index - 1][pattern_index] || matches[name_index][pattern_index - 1]
                }
                '?' => matches[name_index - 1][pattern_index - 1],
                character => {
                    character == name[name_index - 1] && matches[name_index - 1][pattern_index - 1]
                }
            };
        }
    }
    matches[name.len()][pattern.len()]
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
