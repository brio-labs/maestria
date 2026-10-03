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

/// Access-token shape prefixes classified as `AccessToken`. Mirrors the
/// `_SECRET_ACCESS_TOKEN_PATTERN` vocabulary in `scripts/philosophy-check.py`.
const ACCESS_TOKEN_PREFIXES: &[&str] =
    &["AKIA", "ghp_", "github_pat_", "xoxb-", "xoxp-", "sk_live_"];

/// Credential assignment keys classified as `CredentialAssignment`. Mirrors
/// `_SECRET_ASSIGNMENT_KEYS` in `scripts/philosophy-check.py`.
const CREDENTIAL_KEYS: &[&str] = &["password", "passwd", "api_key", "apikey", "secret", "token"];

/// Scan text before indexing, embedding, exporting, or sending it to a provider.
pub fn scan_secrets(text: &str) -> SecretScan {
    let mut findings = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let kind = if trimmed.contains("-----BEGIN ") && trimmed.contains(" PRIVATE KEY-----") {
            Some(SecretKind::PrivateKey)
        } else if ACCESS_TOKEN_PREFIXES
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
    CREDENTIAL_KEYS.iter().any(|key| {
        normalized_name.eq_ignore_ascii_case(key)
            && !value
                .trim()
                .trim_matches(|character| matches!(character, '"' | '\'' | ',' | '}'))
                .trim()
                .is_empty()
    })
}
