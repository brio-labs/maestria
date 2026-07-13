use maestria_governance::Scope;
use maestria_ports::PortError;
use std::path::PathBuf;

/// Allowed shell commands for governed harness execution.
const ALLOWED_COMMANDS: &[&str] = &["echo", "pwd", "cat"];

/// Prohibited shell metacharacters and redirection operators.
const PROHIBITED_CHARS: &[char] = &[
    '|', '&', ';', '$', '`', '(', ')', '{', '}', '<', '>', '\\', '!', '~', '*', '?',
];

/// Returns `true` when `command` uses only the allowed grammar:
/// - starts with `echo`, `pwd`, or `cat`
/// - contains no shell metacharacters, redirection, or newlines
pub(crate) fn is_shell_grammar_allowed(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Reject embedded newlines
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return false;
    }

    // Reject prohibited metacharacters
    if trimmed.contains(PROHIBITED_CHARS) {
        return false;
    }

    // Must start with an allowed command word
    let first_word = trimmed.split_ascii_whitespace().next().map_or("", |w| w);
    ALLOWED_COMMANDS.contains(&first_word)
}

/// Extract the path arguments from a `cat` command.
/// Skips option tokens (leading `-`) and the `--` separator.
/// Returns empty vec for non-cat commands.
pub(crate) fn cat_path_args(command: &str) -> Vec<&str> {
    let trimmed = command.trim();
    let mut tokens = trimmed.split_ascii_whitespace();
    match tokens.next() {
        Some("cat") => tokens.filter(|t| !t.starts_with('-')).collect(),
        _ => Vec::new(),
    }
}
/// Determine a working directory contained within the configured scope.
/// An unrestricted scope still requires a valid process working directory.
pub(crate) fn resolve_working_directory(scope: &Scope) -> Result<PathBuf, PortError> {
    if let Some(root) = scope.readable_roots().first() {
        return Ok(root.clone());
    }
    std::env::current_dir().map_err(|error| PortError::InvalidInput {
        message: format!("unable to resolve unrestricted harness working directory: {error}"),
    })
}
