use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use crate::shell_policy::{cat_path_args, is_shell_grammar_allowed, resolve_working_directory};
use maestria_domain::QueryHarnessRequest;
use maestria_ports::HarnessCommandClass;
use std::path::{Path, PathBuf};

impl EffectExecutionContext {
    /// Gate a harness request against capability, scope, and shell grammar
    /// policy before execution, resolving the harness working directory.
    pub(super) fn gate_harness_request(
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
        let scope = &self.scope;
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
                if let Err(containment_err) = scope.check_read_containment(&path) {
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
}

fn resolve_cat_path(raw_path: &str, working_directory: &Path) -> PathBuf {
    let path = Path::new(raw_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_directory.join(path)
    }
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
