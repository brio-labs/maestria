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

/// Resolves symlinks in the existing prefix of a path and preserves any
/// non-existent suffix. This keeps scope checks correct for paths such as
/// `link/new-file`, where `link` is a symlink but the final file is not yet
/// present.
pub(crate) fn canonicalize_existing_prefix(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let current = std::env::current_dir().ok()?;
        current.join(path)
    };
    let mut candidate = absolute;
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(&candidate) {
            Ok(real) => {
                let mut resolved = real;
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Some(normalize_path(&resolved));
            }
            Err(_) => {
                let name = candidate.file_name()?.to_os_string();
                missing.push(name);
                if !candidate.pop() {
                    return None;
                }
            }
        }
    }
}

/// Resolve a caller-supplied path against the current working directory,
/// failing with a typed error when the working directory is unavailable
/// (R24): a blocked-path check must never silently compare a relative path
/// against absolute policy paths.
pub(crate) fn normalize_absolute_path(path: &Path) -> Result<PathBuf, PortError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let current = std::env::current_dir().map_err(|error| PortError::InternalContext {
            context: "resolve relative path against working directory",
            source: format!("{}: {error}", path.display()),
        })?;
        current.join(path)
    };
    Ok(normalize_path(&absolute))
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

pub(crate) struct AuthorizedPaths {
    pub(crate) working_directory: PathBuf,
    pub(crate) readable_roots: Vec<PathBuf>,
    pub(crate) blocked_paths: Vec<PathBuf>,
    #[cfg(target_os = "linux")]
    pub(crate) blocked_relative_paths: Vec<Vec<PathBuf>>,
    #[cfg(target_os = "linux")]
    pub(crate) blocked_path_handles: Vec<std::fs::File>,
    #[cfg(target_os = "linux")]
    pub(crate) readable_root_handles: Vec<std::fs::File>,
}
#[cfg(target_os = "linux")]
fn open_readable_root(root: &Path) -> Result<(PathBuf, std::fs::File), PortError> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY)
        .open(root)
        .map_err(|error| PortError::InvalidInputContext {
            context: "validate harness readable root",
            source: format!("{}: {error}", root.display()),
        })?;
    use std::os::fd::AsRawFd;
    let identity =
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|error| {
            PortError::InternalContext {
                context: "verify harness readable root identity",
                source: error.to_string(),
            }
        })?;
    let identity_string = identity.to_string_lossy();
    if identity_string.ends_with(" (deleted)") || !identity.is_absolute() {
        return Err(PortError::InternalContext {
            context: "verify harness readable root identity",
            source: format!("opened root has unstable identity {}", identity.display()),
        });
    }
    Ok((normalize_path(&identity), file))
}

#[cfg(target_os = "linux")]
fn open_blocked_path(path: &Path) -> Result<Option<std::fs::File>, PortError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_PATH)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(PortError::InvalidInputContext {
                context: "pin harness blocked path",
                source: format!("{}: {error}", path.display()),
            });
        }
    };
    let identity =
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|error| {
            PortError::InternalContext {
                context: "verify harness blocked path identity",
                source: error.to_string(),
            }
        })?;
    if normalize_path(&identity) != path {
        return Err(PortError::InvalidInputContext {
            context: "pin harness blocked path",
            source: format!("{} changed identity during authorization", path.display()),
        });
    }
    Ok(Some(file))
}

/// Authorize path policy once. Execution later checks opened file identities
/// against these canonical policy paths; it never re-resolves caller paths.
pub(crate) fn authorize_paths(request: &HarnessRequest) -> Result<AuthorizedPaths, PortError> {
    let working_directory = std::fs::canonicalize(&request.working_directory).map_err(|error| {
        PortError::InvalidInputContext {
            context: "validate harness working directory",
            source: format!("{}: {error}", request.working_directory.display()),
        }
    })?;
    if !working_directory.is_dir() {
        return Err(PortError::InvalidInputContext {
            context: "validate harness working directory",
            source: format!("{} is not a directory", working_directory.display()),
        });
    }

    #[cfg(target_os = "linux")]
    let (readable_roots, readable_root_handles) = request
        .readable_roots
        .iter()
        .map(|root| open_readable_root(root))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .unzip::<PathBuf, std::fs::File, Vec<_>, Vec<_>>();
    #[cfg(not(target_os = "linux"))]
    let readable_roots = request
        .readable_roots
        .iter()
        .map(|root| {
            root.canonicalize()
                .map_err(|error| PortError::InvalidInputContext {
                    context: "validate harness readable root",
                    source: format!("{}: {error}", root.display()),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    #[cfg(not(target_os = "linux"))]
    if readable_roots.iter().any(|root| !root.is_dir()) {
        return Err(PortError::InvalidInputContext {
            context: "validate harness readable root",
            source: "readable roots must be directories".to_string(),
        });
    }
    if !readable_roots
        .iter()
        .any(|root| working_directory.starts_with(root))
    {
        return Err(PortError::InvalidInputContext {
            context: "working directory outside readable roots",
            source: working_directory.display().to_string(),
        });
    }

    let blocked_paths = request
        .blocked_paths
        .iter()
        .map(|path| {
            // Canonicalize the existing prefix, falling back to a lexical
            // normalize for paths that do not exist yet. A missing working
            // directory is a typed error (R24): the exclusion check must
            // never silently compare a relative path against absolute policy
            // paths.
            match canonicalize_existing_prefix(path) {
                Some(resolved) => Ok(resolved),
                None => normalize_absolute_path(path),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if blocked_paths
        .iter()
        .any(|blocked| working_directory.starts_with(blocked))
    {
        return Err(PortError::InvalidInputContext {
            context: "working directory blocked by exclusion",
            source: working_directory.display().to_string(),
        });
    }

    #[cfg(target_os = "linux")]
    let blocked_path_handles = blocked_paths
        .iter()
        .filter_map(|path| open_blocked_path(path).transpose())
        .collect::<Result<Vec<_>, _>>()?;

    #[cfg(target_os = "linux")]
    let blocked_relative_paths = readable_roots
        .iter()
        .map(|root| {
            blocked_paths
                .iter()
                .filter_map(|blocked| blocked.strip_prefix(root).ok().map(Path::to_path_buf))
                .collect()
        })
        .collect();

    Ok(AuthorizedPaths {
        working_directory,
        readable_roots,
        blocked_paths,
        #[cfg(target_os = "linux")]
        blocked_relative_paths,
        #[cfg(target_os = "linux")]
        blocked_path_handles,
        #[cfg(target_os = "linux")]
        readable_root_handles,
    })
}

pub(crate) fn validate_cat_args(
    program: &str,
    argv: &[String],
    request: &HarnessRequest,
) -> Result<Vec<String>, PortError> {
    if program != "cat" {
        return Ok(argv.iter().skip(1).cloned().collect());
    }
    if argv.len() == 1 {
        return Err(PortError::InvalidInputContext {
            context: "validate cat command operands",
            source: "cat requires at least one path operand".to_string(),
        });
    }
    for arg in &argv[1..] {
        if arg.starts_with('-') {
            return Err(PortError::InvalidInputContext {
                context: "cat option not allowed",
                source: arg.to_string(),
            });
        }
        // Lexical policy is checked before opening any operand. Canonical
        // policy is checked against each pinned file identity in process.rs.
        validate_filename_patterns(arg, &request.blocked_patterns)?;
    }
    Ok(argv[1..].to_vec())
}
pub(crate) fn validate_command_args(
    program: &str,
    argv: &[String],
    request: &HarnessRequest,
) -> Result<Vec<String>, PortError> {
    match program {
        "cat" => validate_cat_args(program, argv, request),
        "pwd" if argv.len() > 1 => Err(PortError::InvalidInputContext {
            context: "pwd operands are not allowed",
            source: argv[1..].join(" "),
        }),
        _ => Ok(argv.iter().skip(1).cloned().collect()),
    }
}
