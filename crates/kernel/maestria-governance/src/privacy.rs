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
    pub fn with_extension(mut self, extension: impl Into<String>) -> Self {
        self.sensitive_extensions.push(extension.into());
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

    /// Return the number of configured sensitive names.
    pub fn name_count(&self) -> usize {
        self.sensitive_names.len()
    }

    /// Return the number of configured sensitive extensions.
    pub fn extension_count(&self) -> usize {
        self.sensitive_extensions.len()
    }
}

impl Default for PrivacyExclusions {
    /// Default sensitive set suitable for local-source indexing:
    ///
    /// **Names:** `.env`, `.git`, `.svn`, `.hg`, `credentials`, `credential`,
    /// `secrets`, `secret`, `tokens`, `token`, `passwords`, `password`,
    /// `private_key`, `secret_key`, `authorized_keys`,
    /// `id_rsa`, `id_ed25519`, `id_ecdsa`.
    ///
    /// **Extensions:** `pem`, `key`, `pfx`, `p12`, `jks`, `keystore`, `env`.
    fn default() -> Self {
        Self {
            sensitive_names: vec![
                ".env".into(),
                ".git".into(),
                ".svn".into(),
                ".hg".into(),
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

/// Deterministic content scanner for credentials and high-risk secret material.
///
/// The scanner deliberately returns only classifications and line numbers. It
/// never stores or formats the matched value, so callers can log findings
/// without leaking the secret they are protecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFinding {
    pub kind: SecretKind,
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    PrivateKey,
    AccessToken,
    CredentialAssignment,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SecretScan {
    pub findings: Vec<SecretFinding>,
}

impl SecretScan {
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Scan text before indexing, embedding, exporting, or sending it to a provider.
pub fn scan_secrets(text: &str) -> SecretScan {
    let mut findings = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let kind = if trimmed.contains("-----BEGIN ") && trimmed.contains(" PRIVATE KEY-----") {
            Some(SecretKind::PrivateKey)
        } else if ["AKIA", "ghp_", "github_pat_", "xoxb-", "xoxp-", "sk_live_"]
            .iter()
            .any(|prefix| trimmed.contains(prefix))
        {
            Some(SecretKind::AccessToken)
        } else if contains_credential_assignment(trimmed) {
            Some(SecretKind::CredentialAssignment)
        } else {
            None
        };
        if let Some(kind) = kind {
            findings.push(SecretFinding {
                kind,
                line: line_index + 1,
            });
        }
    }
    SecretScan { findings }
}

fn contains_credential_assignment(line: &str) -> bool {
    let assignment = match line.strip_prefix("export") {
        Some(rest) => match rest.chars().next() {
            Some(character) if character.is_whitespace() => rest.trim_start(),
            _ => line,
        },
        None => line,
    };
    let (name, value) = match assignment.split_once('=') {
        Some(pair) => pair,
        None => match assignment.split_once(':') {
            Some(pair) => pair,
            None => return false,
        },
    };
    let normalized_name = name
        .trim()
        .trim_matches(|character| matches!(character, '"' | '\'' | '{' | '}'))
        .trim();
    ["password", "passwd", "api_key", "apikey", "secret", "token"]
        .iter()
        .any(|key| {
            normalized_name.eq_ignore_ascii_case(key)
                && !value
                    .trim()
                    .trim_matches(|character| matches!(character, '"' | '\'' | ',' | '}'))
                    .trim()
                    .is_empty()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_exclusions_never_match() {
        let exclusions = PrivacyExclusions::new();
        assert!(!exclusions.is_excluded(Path::new("/home/user/.env")));
        assert!(!exclusions.is_excluded(Path::new("secret.key")));
    }

    #[test]
    fn default_excludes_sensitive_names() {
        let exclusions = PrivacyExclusions::default();
        assert!(exclusions.is_excluded(Path::new("/src/.env")));
        assert!(exclusions.is_excluded(Path::new(".env")));
        assert!(exclusions.is_excluded(Path::new("/project/.git/config")));
        assert!(exclusions.is_excluded(Path::new("/etc/credentials")));
        assert!(exclusions.is_excluded(Path::new("secrets/db.yaml")));
        assert!(exclusions.is_excluded(Path::new("/home/user/.ssh/id_rsa")));
        assert!(exclusions.is_excluded(Path::new("/home/user/.ssh/authorized_keys")));
    }

    #[test]
    fn default_excludes_sensitive_extensions() {
        let exclusions = PrivacyExclusions::default();
        assert!(exclusions.is_excluded(Path::new("/certs/server.pem")));
        assert!(exclusions.is_excluded(Path::new("tls.key")));
        assert!(exclusions.is_excluded(Path::new("/etc/ssl/bundle.pfx")));
        assert!(exclusions.is_excluded(Path::new("keystore.jks")));
        assert!(exclusions.is_excluded(Path::new("prod.env")));
    }

    #[test]
    fn normal_paths_are_not_excluded() {
        let exclusions = PrivacyExclusions::default();
        assert!(!exclusions.is_excluded(Path::new("/src/main.rs")));
        assert!(!exclusions.is_excluded(Path::new("/docs/readme.md")));
        assert!(!exclusions.is_excluded(Path::new("Cargo.toml")));
        assert!(!exclusions.is_excluded(Path::new("/home/user/config.json")));
    }

    #[test]
    fn custom_exclusions_work() {
        let exclusions = PrivacyExclusions::new()
            .with_name("classified")
            .with_extension("secret");
        assert!(exclusions.is_excluded(Path::new("/docs/classified/report.txt")));
        assert!(exclusions.is_excluded(Path::new("notes.secret")));
        assert!(!exclusions.is_excluded(Path::new("/docs/public/report.txt")));
    }

    #[test]
    fn empty_path_is_not_excluded() {
        let exclusions = PrivacyExclusions::default();
        assert!(!exclusions.is_excluded(Path::new("")));
    }
    #[test]
    fn secret_scan_classifies_without_retaining_values() {
        let scan = scan_secrets(
            "password=super-secret-value\n-----BEGIN PRIVATE KEY-----\nAKIA1234567890",
        );
        assert_eq!(scan.findings.len(), 3);
        assert_eq!(scan.findings[0].kind, SecretKind::CredentialAssignment);
        assert_eq!(scan.findings[0].line, 1);
        assert_eq!(scan.findings[1].kind, SecretKind::PrivateKey);
        assert_eq!(scan.findings[2].kind, SecretKind::AccessToken);
        assert!(!format!("{scan:?}").contains("super-secret-value"));
    }

    #[test]
    fn secret_scan_detects_exported_credentials() {
        let scan = scan_secrets("export API_KEY = value\nexported_token = prose");
        assert_eq!(scan.findings.len(), 1);
        assert_eq!(scan.findings[0].kind, SecretKind::CredentialAssignment);
    }

    #[test]
    fn secret_scan_detects_structured_credentials() {
        let scan = scan_secrets("api_key: value\n{\"password\":\"value\"}");
        assert_eq!(scan.findings.len(), 2);
    }

    #[test]
    fn secret_scan_allows_normal_text() {
        assert!(scan_secrets("passwords are rotated regularly").is_clean());
    }
}
