use std::{
    fs,
    path::{Path, PathBuf},
};

use super::super::protocol::{
    ClientResponse, FederationCredential, RealmGrantAccess, RealmGrantCreatedResponse,
    RealmGrantListResponse, RealmGrantResponse, RealmGrantSensitivity,
};
use super::super::server::{ApiContext, RequestPrincipal};
use anyhow::{Result, anyhow, bail};
use maestria_domain::{
    DomainInput, FederatedEvidenceBounds, FederatedReadAccess, GrantTokenDigest,
    IssueRealmReadGrantInput, MAX_REALM_GRANT_ROOT_BYTES, MAX_REALM_GRANT_ROOTS, RealmReadGrant,
    RealmReadGrantExpiry, RealmReadGrantState, RevokeRealmReadGrantInput, Sensitivity,
};
const MAX_REALM_GRANT_TTL_SECONDS: u64 = 365 * 24 * 60 * 60;

pub(super) struct GrantRequest {
    pub(super) consumer_realm: maestria_domain::RealmId,
    pub(super) access: RealmGrantAccess,
    pub(super) max_sensitivity: RealmGrantSensitivity,
    pub(super) allowed_roots: Vec<String>,
    pub(super) max_results: usize,
    pub(super) max_evidence_bytes: usize,
    pub(super) expires_in_seconds: u64,
}

pub(super) async fn create(
    context: &ApiContext,
    principal: &RequestPrincipal,
    request: GrantRequest,
) -> Result<ClientResponse> {
    require_instance(principal)?;
    let GrantRequest {
        consumer_realm,
        access,
        max_sensitivity,
        allowed_roots,
        max_results,
        max_evidence_bytes,
        expires_in_seconds,
    } = request;
    let allowed_roots = resolve_allowed_roots(context, &allowed_roots)?;
    if !(1..=MAX_REALM_GRANT_TTL_SECONDS).contains(&expires_in_seconds) {
        return Err(anyhow!(
            "grant expiry must be between 1 and {MAX_REALM_GRANT_TTL_SECONDS} seconds"
        ));
    }
    let expires_at_unix_seconds = unix_time_seconds()?
        .checked_add(expires_in_seconds)
        .ok_or_else(|| anyhow!("grant expiry timestamp overflow"))?;
    let expires_at =
        RealmReadGrantExpiry::new(expires_at_unix_seconds).map_err(|error| anyhow!(error))?;
    let bounds = FederatedEvidenceBounds::try_new(max_results, max_evidence_bytes)
        .map_err(|error| anyhow!(error))?;
    let credential = generate_credential()?;
    let grant = RealmReadGrant::new(
        GrantTokenDigest::derive(credential.as_str().as_bytes()),
        context.realm_id.clone(),
        consumer_realm,
        domain_access(access),
        domain_sensitivity(max_sensitivity),
        bounds,
        expires_at,
    )
    .with_allowed_roots(allowed_roots);
    runtime(context)?
        .submit_durable(DomainInput::IssueRealmReadGrant(IssueRealmReadGrantInput {
            grant: grant.clone(),
        }))
        .await
        .map_err(|error| anyhow!(error))?;
    let now = unix_time_seconds()?;
    Ok(ClientResponse::RealmGrantCreated(
        RealmGrantCreatedResponse {
            grant: response_from_grant(&grant, now),
            credential,
        },
    ))
}

fn resolve_allowed_roots(context: &ApiContext, requested: &[String]) -> Result<Vec<PathBuf>> {
    let approved = context.source_manifest.read().read_roots.clone();
    if approved.len() > MAX_REALM_GRANT_ROOTS {
        bail!("at most {MAX_REALM_GRANT_ROOTS} approved roots are supported for grants");
    }
    if approved.is_empty() {
        bail!("realm grants require at least one currently approved read root");
    }

    let capacity = if requested.is_empty() {
        approved.len()
    } else {
        requested.len().min(MAX_REALM_GRANT_ROOTS)
    };
    let mut allowed_roots = Vec::with_capacity(capacity);
    if requested.is_empty() {
        for path in approved {
            let root = canonical_grant_root(&path)?;
            if !allowed_roots.contains(&root) {
                allowed_roots.push(root);
            }
        }
    } else {
        for path in requested {
            let path = PathBuf::from(path);
            let root = canonical_grant_root(&path)?;
            if !approved.contains(&root) {
                bail!(
                    "realm grant root is not currently approved: {}",
                    path.display()
                );
            }
            if !allowed_roots.contains(&root) {
                allowed_roots.push(root);
            }
        }
    }
    if allowed_roots.len() > MAX_REALM_GRANT_ROOTS {
        bail!("at most {MAX_REALM_GRANT_ROOTS} roots may be allowed for a grant");
    }
    if allowed_roots
        .iter()
        .map(|root| root.as_os_str().len())
        .sum::<usize>()
        > MAX_REALM_GRANT_ROOT_BYTES
    {
        bail!("realm grant root paths exceed {MAX_REALM_GRANT_ROOT_BYTES} bytes");
    }
    if allowed_roots.is_empty() {
        bail!("realm grants require at least one currently approved read root");
    }
    Ok(allowed_roots)
}

fn canonical_grant_root(path: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| anyhow!("inspect realm grant root {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "realm grant root must not be a symbolic link: {}",
            path.display()
        );
    }
    if !metadata.is_dir() {
        bail!("realm grant root is not a directory: {}", path.display());
    }
    path.canonicalize()
        .map_err(|error| anyhow!("canonicalize realm grant root {}: {error}", path.display()))
}

pub(super) fn list(context: &ApiContext, principal: &RequestPrincipal) -> Result<ClientResponse> {
    require_instance(principal)?;
    let now = unix_time_seconds()?;
    let mut grants = runtime(context)?
        .realm_read_grant_repository()
        .list()
        .map_err(|error| anyhow!(error))?;
    grants.sort_by(|left, right| {
        left.token_digest()
            .as_str()
            .cmp(right.token_digest().as_str())
    });
    Ok(ClientResponse::RealmGrantList(RealmGrantListResponse {
        grants: grants
            .iter()
            .map(|grant| response_from_grant(grant, now))
            .collect(),
    }))
}

pub(super) async fn revoke(
    context: &ApiContext,
    principal: &RequestPrincipal,
    token_digest: String,
) -> Result<ClientResponse> {
    require_instance(principal)?;
    let token_digest = GrantTokenDigest::try_from(token_digest).map_err(|error| anyhow!(error))?;
    runtime(context)?
        .submit_durable(DomainInput::RevokeRealmReadGrant(
            RevokeRealmReadGrantInput { token_digest },
        ))
        .await
        .map_err(|error| anyhow!(error))?;
    Ok(ClientResponse::RealmGrantList(RealmGrantListResponse {
        grants: Vec::new(),
    }))
}

fn runtime(context: &ApiContext) -> Result<&maestria_runtime::RuntimeHandle> {
    context
        .runtime
        .as_ref()
        .ok_or_else(|| anyhow!("realm federation requires a live daemon runtime"))
}

fn require_instance(principal: &RequestPrincipal) -> Result<()> {
    if matches!(principal, RequestPrincipal::Instance) {
        Ok(())
    } else {
        Err(anyhow!(
            "realm grant administration requires instance authentication"
        ))
    }
}

fn generate_credential() -> Result<FederationCredential> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow!("generate federation credential: {error}"))?;
    let value = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    FederationCredential::try_from(value)
}

pub(super) fn response_from_grant(
    grant: &RealmReadGrant,
    now_unix_seconds: u64,
) -> RealmGrantResponse {
    RealmGrantResponse {
        token_digest: grant.token_digest().as_str().to_string(),
        provider_realm: grant.provider_realm().clone(),
        consumer_realm: grant.consumer_realm().clone(),
        access: protocol_access(grant.access()),
        max_sensitivity: protocol_sensitivity(grant.max_sensitivity()),
        allowed_roots: grant.allowed_roots().map(|roots| {
            roots
                .iter()
                .map(|root| root.display().to_string())
                .collect()
        }),
        max_results: grant.bounds().max_results(),
        max_evidence_bytes: grant.bounds().max_evidence_bytes(),
        expires_at_unix_seconds: grant.expires_at().unix_seconds(),
        state: match grant.state() {
            RealmReadGrantState::Revoked => "revoked",
            RealmReadGrantState::Active if grant.expires_at().is_expired_at(now_unix_seconds) => {
                "expired"
            }
            RealmReadGrantState::Active => "active",
        }
        .to_owned(),
    }
}

fn unix_time_seconds() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| anyhow!("read grant expiry clock: {error}"))?
        .as_secs())
}

fn domain_access(access: RealmGrantAccess) -> FederatedReadAccess {
    match access {
        RealmGrantAccess::SearchOnly => FederatedReadAccess::SearchOnly,
        RealmGrantAccess::SearchAndOpenEvidence => FederatedReadAccess::SearchAndOpenEvidence,
    }
}

fn protocol_access(access: FederatedReadAccess) -> RealmGrantAccess {
    match access {
        FederatedReadAccess::SearchOnly => RealmGrantAccess::SearchOnly,
        FederatedReadAccess::SearchAndOpenEvidence => RealmGrantAccess::SearchAndOpenEvidence,
    }
}

fn domain_sensitivity(sensitivity: RealmGrantSensitivity) -> Sensitivity {
    match sensitivity {
        RealmGrantSensitivity::Public => Sensitivity::Public,
        RealmGrantSensitivity::Internal => Sensitivity::Internal,
        RealmGrantSensitivity::Confidential => Sensitivity::Confidential,
        RealmGrantSensitivity::Restricted => Sensitivity::Restricted,
    }
}

fn protocol_sensitivity(sensitivity: &Sensitivity) -> RealmGrantSensitivity {
    match sensitivity {
        Sensitivity::Public => RealmGrantSensitivity::Public,
        Sensitivity::Internal => RealmGrantSensitivity::Internal,
        Sensitivity::Confidential => RealmGrantSensitivity::Confidential,
        Sensitivity::Restricted => RealmGrantSensitivity::Restricted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[tokio::test]
    async fn create_rejects_unapproved_root_before_runtime_access() -> Result<()> {
        let temp_dir = TempDir::create()?;
        let approved_root = temp_dir.path().join("approved");
        let outside_root = temp_dir.path().join("outside");
        fs::create_dir_all(&approved_root)?;
        fs::create_dir_all(&outside_root)?;

        let provider_realm = maestria_test_support::realm_id(10)?;
        let mut manifest = maestria_core::InstanceManifest::default_for_root(
            temp_dir.path().to_path_buf(),
            provider_realm.clone(),
        );
        manifest.read_roots = vec![approved_root.canonicalize()?];
        let context = ApiContext {
            layout: maestria_core::InstanceLayout::for_root(temp_dir.path().to_path_buf()),
            token: "test-token".to_string(),
            socket_path: PathBuf::new(),
            runtime: None,
            realm_id: provider_realm,
            source_manifest: std::sync::Arc::new(parking_lot::RwLock::new(manifest)),
            interactive_searches: std::sync::Arc::new(
                super::super::super::server::InteractiveSearchCoordinator::default(),
            ),
        };

        let error = create(
            &context,
            &RequestPrincipal::Instance,
            GrantRequest {
                consumer_realm: maestria_test_support::realm_id(11)?,
                access: RealmGrantAccess::SearchOnly,
                max_sensitivity: RealmGrantSensitivity::Public,
                allowed_roots: vec![outside_root.display().to_string()],
                max_results: 1,
                max_evidence_bytes: 1,
                expires_in_seconds: 60,
            },
        )
        .await
        .err()
        .ok_or_else(|| anyhow!("unapproved root unexpectedly received a grant"))?;

        assert!(error.to_string().contains("not currently approved"));
        Ok(())
    }
}
