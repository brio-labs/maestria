use std::path::Path;

/// Privacy-sensitive name and extension exclusions.
///
/// `PrivacyExclusions` is a deterministic, side-effect-free policy object
/// that callers can use **before** a filesystem read to decide whether a
/// path matches a known-sensitive name or extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyExclusions {
    /// File or directory names that are always excluded.
    sensitive_names: Vec<String>,
    /// File extensions (without leading dot) that are excluded.
    sensitive_extensions: Vec<String>,
}

impl PrivacyExclusions {
    /// Create an empty exclusion set.
    pub fn new() -> Self {
        Self {
            sensitive_names: Vec::new(),
            sensitive_extensions: Vec::new(),
        }
    }

    /// Add a sensitive file or directory name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.sensitive_names.push(name.into());
        self
    }

    /// Add a sensitive file extension (without leading dot, e.g. `"pem"`).
    ///
    /// A leading dot is normalized away so `".pem"` and `"pem"` configure
    /// the same exclusion; extension matching compares against the path's
    /// extension, which never carries the dot.
    pub fn with_extension(mut self, extension: impl Into<String>) -> Self {
        let extension = extension.into();
        let extension = match extension.strip_prefix('.') {
            Some(stripped) => stripped.to_string(),
            None => extension,
        };
        self.sensitive_extensions.push(extension);
        self
    }

    /// Return `true` when any component of `path` matches a sensitive name
    /// or the file stem's extension matches a sensitive extension.
    ///
    /// The check is purely lexical — no filesystem I/O is performed.
    pub fn is_excluded(&self, path: &Path) -> bool {
        if self.sensitive_names.is_empty() && self.sensitive_extensions.is_empty() {
            return false;
        }

        // Check every path component against the sensitive-name list.
        for component in path.components() {
            if let Some(os_str) = component.as_os_str().to_str()
                && self.sensitive_names.iter().any(|name| name == os_str)
            {
                return true;
            }
        }

        // Check the file extension against the sensitive-extension list.
        if let Some(extension) = path.extension().and_then(|value| value.to_str())
            && self
                .sensitive_extensions
                .iter()
                .any(|item| item == extension)
        {
            return true;
        }

        false
    }
    /// Return the configured sensitive names.
    pub fn sensitive_names(&self) -> &[String] {
        &self.sensitive_names
    }

    /// Return the configured sensitive extensions (without leading dot).
    pub fn sensitive_extensions(&self) -> &[String] {
        &self.sensitive_extensions
    }

    /// Privacy-driven blocked patterns: sensitive names plus `*.ext` forms of
    /// sensitive extensions. Single-sourced for daemon and harness so the
    /// composition cannot drift.
    pub fn privacy_blocked_patterns(&self) -> Vec<String> {
        let mut patterns = self.sensitive_names().to_vec();
        patterns.extend(
            self.sensitive_extensions()
                .iter()
                .map(|extension| format!("*.{extension}")),
        );
        patterns
    }
    /// Whether any component of `path` matches the shared walk-excluded name
    /// set (`.env.*` matched as a prefix) or this instance's privacy set.
    pub fn is_walk_excluded(&self, path: &Path) -> bool {
        path.components().any(|component| {
            component.as_os_str().to_str().is_some_and(|name| {
                WALK_EXCLUDED_NAMES
                    .iter()
                    .any(|candidate| match *candidate {
                        ".env.*" => name.starts_with(".env."),
                        exact => name == exact,
                    })
            })
        }) || self.is_excluded(path)
    }
}

/// Names excluded by every source walk: VCS directories, machine state,
/// secret-bearing names, and build output. `.env.*` is matched as a prefix
/// convention.
pub const WALK_EXCLUDED_NAMES: &[&str] = &[
    ".git",
    ".ssh",
    ".gnupg",
    "secrets",
    "node_modules",
    "target",
    "dist",
    "build",
    ".env.*",
];

impl Default for PrivacyExclusions {
    /// Default sensitive set suitable for local-source indexing:
    ///
    /// **Names:** `.env`, `.git`, `.svn`, `.hg`, `credentials`, `credential`,
    /// `secrets`, `secret`, `tokens`, `token`, `passwords`, `password`,
    /// `private_key`, `secret_key`, `authorized_keys`, `id_rsa`, `id_ed25519`,
    /// `id_ecdsa`.
    ///
    /// **Machine-state directories:** the XDG Base Directory defaults
    /// (`~/.cache`, `~/.config`), tool homes (`~/.cargo`, `~/.rustup`,
    /// `~/.nvm`, `~/.var`), and dependency/build artifacts (`vendor`,
    /// `.venv`, `__pycache__`). These are regenerable machine state, never
    /// user content; per-user bulk directories (downloads, tool dumps) are
    /// an instance-level policy on top of this set.
    ///
    /// **Extensions:** `pem`, `key`, `pfx`, `p12`, `jks`, `keystore`, `env`.
    fn default() -> Self {
        Self {
            sensitive_names: vec![
                ".env".into(),
                ".git".into(),
                ".svn".into(),
                ".hg".into(),
                ".cache".into(),
                ".config".into(),
                ".cargo".into(),
                ".rustup".into(),
                ".nvm".into(),
                ".var".into(),
                "vendor".into(),
                ".venv".into(),
                "__pycache__".into(),
                "credentials".into(),
                "credential".into(),
                "secrets".into(),
                "secret".into(),
                "tokens".into(),
                "token".into(),
                "passwords".into(),
                "password".into(),
                "private_key".into(),
                "secret_key".into(),
                "authorized_keys".into(),
                "id_rsa".into(),
                "id_ed25519".into(),
                "id_ecdsa".into(),
            ],
            sensitive_extensions: vec![
                "pem".into(),
                "key".into(),
                "pfx".into(),
                "p12".into(),
                "jks".into(),
                "keystore".into(),
                "env".into(),
            ],
        }
    }
}
