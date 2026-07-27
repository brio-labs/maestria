use maestria_ports::{HarnessRequest, PortError};
use std::path::{Component, Path, PathBuf};

pub(crate) const FORBIDDEN_CHARS: &[char] = &[
    '|', '&', ';', '<', '>', '$', '`', '\\', '(', ')', '{', '}', '[', ']', '!', '~', '#', '*', '?',
];

pub(crate) const ALLOWED_PROGRAMS: &[&str] = &["echo", "pwd", "cat"];

pub(crate) fn reject_metachar(arg: &str) -> Result<(), PortError> {
    if arg.find(FORBIDDEN_CHARS).is_some() {
        return Err(PortError::InvalidInputContext {
            context: "forbidden metacharacter in command argument",
            source: arg.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_readable_path(
    raw_path: &str,
    cwd: &Path,
    readable_roots: &[PathBuf],
    blocked_paths: &[PathBuf],
) -> Result<PathBuf, PortError> {
    let candidate = if Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else {
        cwd.join(raw_path)
    };
    let normalized = normalize_path(&candidate);
    if blocked_paths.iter().any(|b| normalized.starts_with(b)) {
        return Err(PortError::InvalidInputContext {
            context: "path is blocked by exclusion",
            source: raw_path.to_string(),
        });
    }
    // Canonicalize the candidate to resolve symlinks, then re-check.
    if let Ok(real) = std::fs::canonicalize(&candidate) {
        let real_allowed = readable_roots.iter().any(|root| match root.canonicalize() {
            Ok(cr) => real.starts_with(&cr),
            Err(_) => false,
        });
        if !real_allowed {
            return Err(PortError::InvalidInputContext {
                context: "path resolves outside readable roots",
                source: format!("{raw_path:?} -> {real:?}"),
            });
        }
        return Ok(real);
    }
    // Candidate does not exist — fall back to lexical check.
    let allowed = readable_roots.iter().any(|root| match root.canonicalize() {
        Ok(cr) => normalized.starts_with(&cr),
        Err(_) => false,
    });
    if !allowed {
        return Err(PortError::InvalidInputContext {
            context: "path is outside readable roots",
            source: raw_path.to_string(),
        });
    }
    Ok(normalized)
}

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut components: Vec<Component<'_>> = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if components
                    .last()
                    .is_some_and(|prev| !matches!(prev, Component::ParentDir))
                {
                    components.pop();
                } else {
                    components.push(c);
                }
            }
            other => components.push(other),
        }
    }
    components.iter().collect()
}

pub(crate) fn filename_matches(name: &str, pattern: &str) -> bool {
    if !pattern.contains('*') && !pattern.contains('?') {
        return name == pattern;
    }
    if pattern == "*" {
        return true;
    }
    glob_match(name, pattern)
}

fn glob_match(name: &str, pattern: &str) -> bool {
    let nc: Vec<char> = name.chars().collect();
    let pc: Vec<char> = pattern.chars().collect();
    let (n, p) = (nc.len(), pc.len());
    let mut dp = vec![vec![false; p + 1]; n + 1];
    dp[0][0] = true;
    for j in 1..=p {
        if pc[j - 1] == '*' {
            dp[0][j] = dp[0][j - 1];
        }
    }
    for i in 1..=n {
        for j in 1..=p {
            if pc[j - 1] == '*' {
                dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
            } else if pc[j - 1] == '?' || pc[j - 1] == nc[i - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            }
        }
    }
    dp[n][p]
}

pub(crate) fn validate_filename_patterns(
    raw_path: &str,
    patterns: &[String],
) -> Result<(), PortError> {
    if patterns.is_empty() {
        return Ok(());
    }
    let path = Path::new(raw_path);
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        for pattern in patterns {
            if filename_matches(&name, pattern) {
                return Err(PortError::InvalidInputContext {
                    context: "path matches blocked pattern",
                    source: format!("{raw_path:?} matches {pattern:?}"),
                });
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_cat_args(
    program: &str,
    argv: &[String],
    request: &HarnessRequest,
) -> Result<(), PortError> {
    if program != "cat" {
        return Ok(());
    }
    let mut has_path_arg = false;
    for arg in &argv[1..] {
        if arg.starts_with('-') {
            return Err(PortError::InvalidInputContext {
                context: "cat option not allowed",
                source: arg.to_string(),
            });
        }
        has_path_arg = true;
    }
    if !has_path_arg {
        return Err(PortError::InvalidInputContext {
            context: "validate cat command operands",
            source: "cat requires at least one path operand".to_string(),
        });
    }
    for arg in &argv[1..] {
        let resolved = validate_readable_path(
            arg,
            &request.working_directory,
            &request.readable_roots,
            &request.blocked_paths,
        )?;
        let check_path = match std::fs::canonicalize(&resolved) {
            Ok(p) => p,
            Err(_) => resolved,
        };
        if request
            .blocked_paths
            .iter()
            .any(|b| check_path.starts_with(b))
        {
            return Err(PortError::InvalidInputContext {
                context: "canonical path blocked by exclusion",
                source: check_path.display().to_string(),
            });
        }
        let path_str = match check_path.to_str() {
            Some(s) => s,
            None => arg,
        };
        validate_filename_patterns(path_str, &request.blocked_patterns)?;
    }
    Ok(())
}
