use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use sillage_daemon::{ClientOperation, ClientResponse, DaemonClient};
use sillage_domain::{MAX_REALM_GRANT_ROOT_BYTES, MAX_REALM_GRANT_ROOTS, RealmId};

use super::{
    ExternalGrantRequest, GrantPolicy, approved_root_argument, display_access, display_sensitivity,
    owner_client, protocol_access, protocol_sensitivity,
};

pub(super) async fn review_external_grant(
    instance_dir: PathBuf,
    request: ExternalGrantRequest,
    consumer_label: String,
) -> Result<()> {
    let ExternalGrantRequest {
        consumer_realm,
        allowed_roots: requested_roots,
        policy,
    } = request;
    let provider_client = owner_client(instance_dir)?;
    let allowed_roots = review_roots(&provider_client, requested_roots).await?;
    ensure_consumer_can_be_granted(&provider_client, &consumer_realm).await?;

    let mut stdout = io::stdout().lock();
    review_external_grant_output(&mut stdout, &consumer_label, &policy, &allowed_roots)
}

async fn review_roots(
    provider_client: &DaemonClient,
    requested: Vec<String>,
) -> Result<Vec<String>> {
    let response = provider_client
        .request(ClientOperation::SearchRootsStatus)
        .await?;
    let ClientResponse::SearchRootsStatus(status) = response else {
        bail!("provider returned an unexpected approved-root status response");
    };
    if status.roots_truncated
        || status.approved_root_count != status.roots.len()
        || status.roots.iter().any(|root| root.path_truncated)
    {
        bail!("provider root inventory is incomplete; refusing to review an ambiguous scope");
    }
    if status.roots.is_empty() {
        bail!("realm grants require at least one currently approved read root");
    }

    let mut approved_roots = Vec::with_capacity(status.roots.len());
    for root in status.roots {
        let canonical = approved_root_argument(Path::new(&root.path))?
            .display()
            .to_string();
        if !approved_roots.contains(&canonical) {
            approved_roots.push(canonical);
        }
    }
    let allowed_roots = if requested.is_empty() {
        approved_roots
    } else {
        let mut allowed_roots = Vec::with_capacity(requested.len().min(MAX_REALM_GRANT_ROOTS));
        for root in requested {
            if !approved_roots.contains(&root) {
                bail!("requested grant root is not currently approved");
            }
            if !allowed_roots.contains(&root) {
                allowed_roots.push(root);
            }
        }
        allowed_roots
    };
    let root_bytes = allowed_roots
        .iter()
        .try_fold(0_usize, |total, root| total.checked_add(root.len()));
    if allowed_roots.len() > MAX_REALM_GRANT_ROOTS
        || root_bytes.is_none_or(|bytes| bytes > MAX_REALM_GRANT_ROOT_BYTES)
    {
        bail!("grant root scope exceeds the supported root-count or path-byte bounds");
    }
    Ok(allowed_roots)
}

async fn ensure_consumer_can_be_granted(
    provider_client: &DaemonClient,
    consumer_realm: &RealmId,
) -> Result<()> {
    let response = provider_client
        .request(ClientOperation::RealmGrantList)
        .await?;
    let ClientResponse::RealmGrantList(list) = response else {
        bail!("provider returned an unexpected grant-list response");
    };
    if list
        .grants
        .iter()
        .any(|grant| &grant.consumer_realm == consumer_realm && grant.state != "revoked")
    {
        bail!("an unrevoked grant already exists for this consumer; revoke it before review");
    }
    Ok(())
}

fn review_external_grant_output(
    stdout: &mut impl Write,
    consumer_label: &str,
    policy: &GrantPolicy,
    allowed_roots: &[String],
) -> Result<()> {
    writeln!(stdout, "External search grant review (no changes made)")?;
    writeln!(stdout, "provider=Local Sillage Search instance")?;
    writeln!(
        stdout,
        "consumer_label={}",
        serde_json::to_string(consumer_label)?
    )?;
    writeln!(
        stdout,
        "access={}",
        display_access(protocol_access(policy.access))
    )?;
    writeln!(
        stdout,
        "max_sensitivity={}",
        display_sensitivity(protocol_sensitivity(policy.max_sensitivity))
    )?;
    for root in allowed_roots {
        writeln!(stdout, "read_root={}", serde_json::to_string(root)?)?;
    }
    writeln!(stdout, "max_results={}", policy.max_results)?;
    writeln!(stdout, "max_evidence_bytes={}", policy.max_evidence_bytes)?;
    writeln!(
        stdout,
        "expiry=TTL starts when explicit create-external action is run"
    )?;
    writeln!(
        stdout,
        "expires_in_seconds_after_creation={}",
        policy.expires_in_seconds
    )?;
    writeln!(
        stdout,
        "approval=run owner grant create-external with the reviewed constraints"
    )?;
    Ok(())
}
