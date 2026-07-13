use maestria_ports::{HarnessRequest, PortError};
use std::path::{Component, Path, PathBuf};

pub(crate) const FORBIDDEN_CHARS: &[char] = &[
    '|', '&', ';', '<', '>', '$', '`', '\\', '(', ')', '{', '}', '[', ']', '!', '~', '#', '*', '?',
];

pub(crate) const ALLOWED_PROGRAMS: &[&str] = &["echo", "pwd", "cat"];

pub(crate) fn reject_metachar(arg: &str) -> Result<(), PortError> {
    if let Some(pos) = arg.find(FORBIDDEN_CHARS) {
        return Err(PortError::InvalidInput {
            message: format!(
                "forbidden metacharacter {:?} at offset {} in {:?}",
                arg.chars().nth(pos).map_or('?', |c| c),
                pos,
                arg
            ),
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
        return Err(PortError::InvalidInput {
            message: format!("path {:?} is blocked by exclusion", raw_path),
        });
    }
    let allowed = readable_roots.iter().any(|root| match root.canonicalize() {
        Ok(cr) => normalized.starts_with(&cr),
        Err(_) => false,
    });
    if !allowed {
        return Err(PortError::InvalidInput {
            message: format!("path {:?} is outside readable roots", raw_path),
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
                return Err(PortError::InvalidInput {
                    message: format!("path {raw_path:?} matches blocked pattern {pattern:?}"),
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
            return Err(PortError::InvalidInput {
                message: format!("cat option {arg:?} not allowed; only path operands"),
            });
        }
        has_path_arg = true;
    }
    if !has_path_arg {
        return Err(PortError::InvalidInput {
            message: "cat requires at least one path operand".to_string(),
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
            return Err(PortError::InvalidInput {
                message: format!("canonical path {:?} is blocked by exclusion", check_path),
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
