use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sillage_extensions::HttpMethod;
use thiserror::Error;
use url::Url;
use zeroize::Zeroizing;

mod policy;
use policy::{request_matches, validate_policy, validate_secret};

#[cfg(target_os = "linux")]
mod secret_service;
mod storage;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpGrantPolicy {
    pub provider_label: String,
    pub extension_id: String,
    pub package_sha256: String,
    pub origin: String,
    pub method: HttpMethod,
    pub path: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HttpGrantState {
    Active,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpGrantMetadata {
    pub handle: String,
    pub policy: HttpGrantPolicy,
    pub created_at: u64,
    pub expires_at: u64,
    pub state: HttpGrantState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationStatus {
    Deleted,
    Pending,
}

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpGrantError {
    #[error("The HTTP credential policy is invalid.")]
    InvalidPolicy,
    #[error("The HTTP bearer credential is invalid.")]
    InvalidSecret,
    #[error("The HTTP credential grant handle is invalid or unknown.")]
    InvalidHandle,
    #[error("The request is outside the approved HTTP credential scope.")]
    Denied,
    #[error("The HTTP credential grant has expired.")]
    Expired,
    #[error("The HTTP credential grant has been revoked.")]
    Revoked,
    #[error("Secure credential storage is unavailable or locked.")]
    Unavailable,
    #[error("The HTTP credential operation failed.")]
    Failed,
}

impl HttpGrantError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidPolicy | Self::InvalidSecret | Self::InvalidHandle => "invalid_request",
            Self::Denied | Self::Expired | Self::Revoked => "permission_denied",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
        }
    }
}

/// An in-memory bearer credential. Formatting is always redacted and its allocation is zeroed
/// when dropped.
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl std::fmt::Debug for SecretBytes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretBytes(<redacted>)")
    }
}

#[derive(Clone)]
pub struct HttpGrantStore {
    root: PathBuf,
}

impl HttpGrantStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Validate and return the public scope for native review. This does not inspect or mutate
    /// persistent state and never accepts a secret.
    pub fn review(&self, policy: &HttpGrantPolicy) -> Result<HttpGrantPolicy, HttpGrantError> {
        validate_policy(policy)?;
        Ok(policy.clone())
    }

    /// Store a newly reviewed credential and publish its independent scope.
    ///
    /// # Cancellation
    /// Dropping this future before metadata publication can leave an unreferenced vault item,
    /// but cannot authorize a request. Native approval runs to completion after navigation.
    pub async fn create(
        &self,
        policy: HttpGrantPolicy,
        secret: SecretBytes,
    ) -> Result<HttpGrantMetadata, HttpGrantError> {
        validate_policy(&policy)?;
        validate_secret(secret.as_bytes())?;
        storage::preflight(&self.root)?;
        let handle = new_handle()?;

        #[cfg(target_os = "linux")]
        let persisted = secret_service::store(&handle, &secret).await;
        #[cfg(not(target_os = "linux"))]
        let persisted: Result<(), HttpGrantError> = Err(HttpGrantError::Unavailable);

        persisted?;

        let (created_at, expires_at) = match unix_time().and_then(|created_at| {
            created_at
                .checked_add(policy.expires_in_seconds)
                .map(|expires_at| (created_at, expires_at))
                .ok_or(HttpGrantError::Failed)
        }) {
            Ok(timestamps) => timestamps,
            Err(error) => {
                #[cfg(target_os = "linux")]
                let _ = secret_service::delete(&handle).await;
                return Err(error);
            }
        };
        let record = storage::StoredGrant {
            handle,
            policy,
            created_at,
            expires_at,
            revoked: false,
        };

        if let Err(error) = storage::insert(&self.root, &record) {
            #[cfg(target_os = "linux")]
            let _ = secret_service::delete(&record.handle).await;
            return Err(error);
        }

        Ok(metadata(record, created_at))
    }

    /// List public metadata only. No Secret Service operation is performed.
    pub fn list(&self) -> Result<Vec<HttpGrantMetadata>, HttpGrantError> {
        let now = unix_time()?;
        let mut grants = storage::read_all(&self.root)?
            .into_iter()
            .map(|record| metadata(record, now))
            .collect::<Vec<_>>();
        grants.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.handle.cmp(&right.handle))
        });
        Ok(grants)
    }

    /// Persist revocation before attempting deletion from the user Secret Service.
    ///
    /// # Cancellation
    /// Once revocation is persisted, cancellation cannot reactivate the grant. Vault deletion
    /// may remain pending; another explicit revocation can finish deletion.
    pub async fn revoke(&self, handle: &str) -> Result<RevocationStatus, HttpGrantError> {
        validate_handle(handle)?;
        storage::revoke(&self.root, handle)?;

        #[cfg(target_os = "linux")]
        {
            return Ok(match secret_service::delete(handle).await {
                Ok(()) => RevocationStatus::Deleted,
                Err(_) => RevocationStatus::Pending,
            });
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(RevocationStatus::Pending)
        }
    }

    /// Resolve an active grant against the already parsed request URL. Metadata is re-read on
    /// every call, and all scope/expiry/revocation checks precede Secret Service access.
    ///
    /// # Cancellation
    /// Cancellation does not mutate grant state. Any retrieved secret is zeroized on drop.
    pub async fn resolve(
        &self,
        extension_id: &str,
        package_sha256: &str,
        handle: &str,
        url: &Url,
        method: HttpMethod,
    ) -> Result<SecretBytes, HttpGrantError> {
        validate_handle(handle)?;
        let record = storage::read_one(&self.root, handle)?.ok_or(HttpGrantError::InvalidHandle)?;
        if record.revoked {
            return Err(HttpGrantError::Revoked);
        }
        let now = unix_time()?;
        if now >= record.expires_at {
            return Err(HttpGrantError::Expired);
        }
        if record.policy.extension_id != extension_id
            || record.policy.package_sha256 != package_sha256
            || !request_matches(&record.policy, url, method)
        {
            return Err(HttpGrantError::Denied);
        }

        #[cfg(target_os = "linux")]
        {
            let secret = secret_service::resolve(handle).await?;
            let current =
                storage::read_one(&self.root, handle)?.ok_or(HttpGrantError::InvalidHandle)?;
            if current.revoked {
                return Err(HttpGrantError::Revoked);
            }
            if unix_time()? >= current.expires_at {
                return Err(HttpGrantError::Expired);
            }
            Ok(secret)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(HttpGrantError::Unavailable)
        }
    }
}

fn metadata(record: storage::StoredGrant, now: u64) -> HttpGrantMetadata {
    let state = if record.revoked {
        HttpGrantState::Revoked
    } else if now >= record.expires_at {
        HttpGrantState::Expired
    } else {
        HttpGrantState::Active
    };
    HttpGrantMetadata {
        handle: record.handle,
        policy: record.policy,
        created_at: record.created_at,
        expires_at: record.expires_at,
        state,
    }
}

fn validate_handle(handle: &str) -> Result<(), HttpGrantError> {
    if !handle.is_empty()
        && handle.len() <= 128
        && handle
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(HttpGrantError::InvalidHandle)
    }
}

fn new_handle() -> Result<String, HttpGrantError> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|_| HttpGrantError::Unavailable)?;
    let mut handle = String::with_capacity(random.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in random {
        handle.push(HEX[(byte >> 4) as usize] as char);
        handle.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(handle)
}

fn unix_time() -> Result<u64, HttpGrantError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| HttpGrantError::Failed)
}
#[cfg(test)]
mod tests {
    use super::*;

    use super::policy::{MAX_SECRET_BYTES, valid_scope_path};

    fn policy() -> HttpGrantPolicy {
        HttpGrantPolicy {
            provider_label: "Example API".to_owned(),
            extension_id: "example.client".to_owned(),
            package_sha256: "a".repeat(64),
            origin: "https://api.example".to_owned(),
            method: HttpMethod::Get,
            path: "/v1/items".to_owned(),
            expires_in_seconds: 300,
        }
    }

    #[test]
    fn policies_and_requests_keep_exact_https_scope() -> Result<(), Box<dyn std::error::Error>> {
        let policy = policy();
        assert!(validate_policy(&policy).is_ok());
        let request = Url::parse("https://api.example/v1/items?page=2")?;
        assert!(request_matches(&policy, &request, HttpMethod::Get));
        assert!(!request_matches(&policy, &request, HttpMethod::Post));

        let nested = Url::parse("https://api.example/v1/items/child")?;
        let other_origin = Url::parse("https://api.example:8443/v1/items")?;
        assert!(!request_matches(&policy, &nested, HttpMethod::Get));
        assert!(!request_matches(&policy, &other_origin, HttpMethod::Get));

        for path in [
            "/v1/../private",
            "/v1/%2fprivate",
            "/v1/%5Cprivate",
            "/v1/%252fprivate",
            "/v1/items?query=1",
            "/v1/items#fragment",
        ] {
            assert!(!valid_scope_path(path));
        }
        assert!(
            validate_policy(&HttpGrantPolicy {
                origin: "https://api.example/".to_owned(),
                ..policy
            })
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn metadata_from_another_handle_cannot_authorize_or_revoke()
    -> Result<(), Box<dyn std::error::Error>> {
        struct OwnedRoot(PathBuf);
        impl Drop for OwnedRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root =
            OwnedRoot(std::env::temp_dir().join(format!("sillage-http-scope-{}", new_handle()?)));
        let original = new_handle()?;
        let substituted = new_handle()?;
        let record = storage::StoredGrant {
            handle: original.clone(),
            policy: policy(),
            created_at: 1,
            expires_at: 301,
            revoked: false,
        };
        storage::insert(&root.0, &record)?;
        std::fs::rename(
            root.0.join(format!("grant-{original}.json")),
            root.0.join(format!("grant-{substituted}.json")),
        )?;
        assert!(matches!(
            storage::read_one(&root.0, &substituted),
            Err(HttpGrantError::Failed)
        ));
        assert_eq!(
            storage::revoke(&root.0, &substituted),
            Err(HttpGrantError::Failed)
        );
        Ok(())
    }

    #[test]
    fn bearer_secret_admission_rejects_header_injection_and_debug_is_redacted() {
        assert!(validate_secret(b"token_A-+.~/==").is_ok());
        assert!(
            validate_secret(&[
                b't', b'o', b'k', b'e', b'n', 0x0d, 0x0a, b'I', b'n', b'j', b'e', b'c', b't',
            ])
            .is_err()
        );
        assert!(validate_secret(b"token value").is_err());
        assert!(validate_secret(&vec![b'a'; MAX_SECRET_BYTES + 1]).is_err());

        let secret = SecretBytes::new(b"private-token".to_vec());
        assert!(!format!("{secret:?}").contains("private-token"));
    }
}
