use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use maestria_core::InstanceLayout;
use maestria_daemon::{
    ClientOperation, ClientResponse, DaemonClient, RealmGrantAccess, RealmGrantSensitivity,
};
use maestria_domain::RealmId;

use super::{
    GrantAccess, GrantCommands, GrantSensitivity, OwnerCommands, RootCommands, parse_realm_id,
};
use crate::consumer::write_credential_file;
pub(super) async fn dispatch_owner(command: OwnerCommands) -> Result<()> {
    match command {
        OwnerCommands::Roots { command } => dispatch_roots(command).await,
        OwnerCommands::Grant { command } => dispatch_grants(command).await,
    }
}

async fn dispatch_roots(command: RootCommands) -> Result<()> {
    let (instance_dir, operation) = match command {
        RootCommands::Status { instance_dir } => (instance_dir, ClientOperation::SearchRootsStatus),
        RootCommands::Add { instance_dir, root } => (
            instance_dir,
            ClientOperation::SearchRootAdd {
                root: approved_root_argument(&root)?.display().to_string(),
            },
        ),
        RootCommands::Remove { instance_dir, root } => (
            instance_dir,
            ClientOperation::SearchRootRemove {
                root: revoked_root_argument(&root)?.display().to_string(),
            },
        ),
    };
    let layout = InstanceLayout::for_root(instance_dir);
    let response = DaemonClient::from_instance(&layout)?
        .request(operation)
        .await?;
    let ClientResponse::SearchRootsStatus(status) = response else {
        bail!("daemon returned a non-root-status response");
    };
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string_pretty(&status)?)?;
    Ok(())
}

async fn dispatch_grants(command: GrantCommands) -> Result<()> {
    match command {
        GrantCommands::CreateExternal {
            instance_dir,
            consumer_realm,
            credential_file,
            access,
            max_sensitivity,
            max_results,
            max_evidence_bytes,
            read_roots,
            expires_in_seconds,
        } => {
            create_external_grant(
                instance_dir,
                parse_realm_id(consumer_realm)?,
                credential_file,
                read_roots,
                GrantPolicy {
                    access,
                    max_sensitivity,
                    max_results,
                    max_evidence_bytes,
                    expires_in_seconds,
                },
            )
            .await
        }
        GrantCommands::List { instance_dir } => list_grants(instance_dir).await,
        GrantCommands::Revoke {
            instance_dir,
            grant_token_digest,
        } => revoke_grant(instance_dir, grant_token_digest).await,
    }
}

struct GrantPolicy {
    access: GrantAccess,
    max_sensitivity: GrantSensitivity,
    max_results: usize,
    max_evidence_bytes: usize,
    expires_in_seconds: u64,
}

async fn create_external_grant(
    instance_dir: PathBuf,
    consumer_realm: RealmId,
    credential_file: PathBuf,
    read_roots: Vec<PathBuf>,
    policy: GrantPolicy,
) -> Result<()> {
    let allowed_roots = read_roots
        .iter()
        .map(|root| approved_root_argument(root).map(|root| root.display().to_string()))
        .collect::<Result<Vec<_>>>()?;
    let provider_client = owner_client(instance_dir)?;
    let created = match provider_client
        .request(ClientOperation::RealmGrantCreate {
            consumer_realm,
            access: protocol_access(policy.access),
            max_sensitivity: protocol_sensitivity(policy.max_sensitivity),
            allowed_roots,
            max_results: policy.max_results,
            max_evidence_bytes: policy.max_evidence_bytes,
            expires_in_seconds: policy.expires_in_seconds,
        })
        .await?
    {
        ClientResponse::RealmGrantCreated(created) => created,
        _ => bail!("provider returned an unexpected realm-grant creation response"),
    };

    if let Err(error) = write_credential_file(&credential_file, created.credential.expose()) {
        let digest = created.grant.token_digest.clone();
        return match provider_client
            .request(ClientOperation::RealmGrantRevoke {
                token_digest: digest.clone(),
            })
            .await
        {
            Ok(ClientResponse::RealmGrantList(_)) => Err(error)
                .context("credential file creation failed; the issued grant was revoked"),
            Ok(_) => Err(error).context(format!(
                "credential file creation failed; revoke provider grant {digest} manually"
            )),
            Err(revoke_error) => Err(error).context(format!(
                "credential file creation failed and grant {digest} could not be revoked: {revoke_error}"
            )),
        };
    }

    let mut stdout = io::stdout().lock();
    print_grant(&mut stdout, &created.grant)?;
    writeln!(stdout, "credential_file={}", credential_file.display())?;
    Ok(())
}

async fn list_grants(instance_dir: PathBuf) -> Result<()> {
    let response = owner_client(instance_dir)?
        .request(ClientOperation::RealmGrantList)
        .await?;
    let ClientResponse::RealmGrantList(list) = response else {
        bail!("provider returned an unexpected realm-grant list response");
    };
    let mut stdout = io::stdout().lock();
    for grant in list.grants {
        writeln!(
            stdout,
            "grant_token_digest={} provider_realm={} consumer_realm={} access={} max_sensitivity={} max_results={} max_evidence_bytes={} expires_at_unix_seconds={} state={} allowed_roots={}",
            grant.token_digest,
            grant.provider_realm.as_str(),
            grant.consumer_realm.as_str(),
            display_access(grant.access),
            display_sensitivity(grant.max_sensitivity),
            grant.max_results,
            grant.max_evidence_bytes,
            grant.expires_at_unix_seconds,
            grant.state,
            display_allowed_roots(&grant.allowed_roots)?,
        )?;
        if grant.expires_at_unix_seconds == 1 {
            writeln!(
                stdout,
                "legacy_grant_reissue_required=true grant_token_digest={}",
                grant.token_digest
            )?;
        }
    }
    Ok(())
}

async fn revoke_grant(instance_dir: PathBuf, token_digest: String) -> Result<()> {
    let response = owner_client(instance_dir)?
        .request(ClientOperation::RealmGrantRevoke {
            token_digest: token_digest.clone(),
        })
        .await?;
    let ClientResponse::RealmGrantList(_) = response else {
        bail!("provider returned an unexpected realm-grant revocation response");
    };
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "grant_token_digest={token_digest}")?;
    writeln!(stdout, "state=revoked")?;
    Ok(())
}

fn owner_client(instance_dir: PathBuf) -> Result<DaemonClient> {
    let layout = InstanceLayout::for_root(instance_dir);
    DaemonClient::from_instance(&layout).context("load daemon owner credential")
}

fn print_grant(stdout: &mut impl Write, grant: &maestria_daemon::RealmGrantResponse) -> Result<()> {
    writeln!(stdout, "grant_token_digest={}", grant.token_digest)?;
    writeln!(stdout, "provider_realm={}", grant.provider_realm.as_str())?;
    writeln!(stdout, "consumer_realm={}", grant.consumer_realm.as_str())?;
    writeln!(stdout, "access={}", display_access(grant.access))?;
    writeln!(
        stdout,
        "max_sensitivity={}",
        display_sensitivity(grant.max_sensitivity)
    )?;
    writeln!(stdout, "max_results={}", grant.max_results)?;
    writeln!(stdout, "max_evidence_bytes={}", grant.max_evidence_bytes)?;
    writeln!(
        stdout,
        "expires_at_unix_seconds={}",
        grant.expires_at_unix_seconds
    )?;
    writeln!(stdout, "state={}", grant.state)?;
    writeln!(
        stdout,
        "allowed_roots={}",
        display_allowed_roots(&grant.allowed_roots)?
    )?;
    Ok(())
}

fn display_allowed_roots(allowed_roots: &Option<Vec<String>>) -> Result<String> {
    match allowed_roots {
        Some(roots) => Ok(serde_json::to_string(roots)?),
        None => Ok("legacy-all-approved".to_string()),
    }
}

fn protocol_access(access: GrantAccess) -> RealmGrantAccess {
    match access {
        GrantAccess::SearchOnly => RealmGrantAccess::SearchOnly,
        GrantAccess::SearchAndOpenEvidence => RealmGrantAccess::SearchAndOpenEvidence,
    }
}

fn protocol_sensitivity(sensitivity: GrantSensitivity) -> RealmGrantSensitivity {
    match sensitivity {
        GrantSensitivity::Public => RealmGrantSensitivity::Public,
        GrantSensitivity::Internal => RealmGrantSensitivity::Internal,
        GrantSensitivity::Confidential => RealmGrantSensitivity::Confidential,
        GrantSensitivity::Restricted => RealmGrantSensitivity::Restricted,
    }
}

fn display_access(access: RealmGrantAccess) -> &'static str {
    match access {
        RealmGrantAccess::SearchOnly => "search-only",
        RealmGrantAccess::SearchAndOpenEvidence => "search-and-open-evidence",
    }
}

fn display_sensitivity(sensitivity: RealmGrantSensitivity) -> &'static str {
    match sensitivity {
        RealmGrantSensitivity::Public => "public",
        RealmGrantSensitivity::Internal => "internal",
        RealmGrantSensitivity::Confidential => "confidential",
        RealmGrantSensitivity::Restricted => "restricted",
    }
}
pub(super) fn approved_root_argument(path: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect approved root {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "approved root must not be a symbolic link: {}",
            path.display()
        );
    }
    if !metadata.is_dir() {
        bail!("approved root is not a directory: {}", path.display());
    }
    path.canonicalize()
        .with_context(|| format!("canonicalize approved root {}", path.display()))
}

fn revoked_root_argument(path: &Path) -> Result<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    let absolute =
        std::path::absolute(path).with_context(|| format!("resolve root {}", path.display()))?;
    maestria_governance::lexical_normalize(&absolute)
        .ok_or_else(|| anyhow!("invalid root path {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_output_displays_scoped_roots_and_legacy_scope() -> Result<()> {
        let grant = maestria_daemon::RealmGrantResponse {
            token_digest: "a".repeat(64),
            provider_realm: RealmId::try_from("a".repeat(64))?,
            consumer_realm: RealmId::try_from("b".repeat(64))?,
            access: RealmGrantAccess::SearchOnly,
            max_sensitivity: RealmGrantSensitivity::Public,
            allowed_roots: Some(vec![
                "/approved/notes".to_string(),
                "/approved/docs".to_string(),
            ]),
            max_results: 1,
            max_evidence_bytes: 1,
            expires_at_unix_seconds: 2,
            state: "active".to_string(),
        };
        let mut output = Vec::new();
        print_grant(&mut output, &grant)?;
        let output = String::from_utf8(output)?;
        assert!(output.contains(r#"allowed_roots=["/approved/notes","/approved/docs"]"#));

        let mut legacy = grant;
        legacy.allowed_roots = None;
        let mut output = Vec::new();
        print_grant(&mut output, &legacy)?;
        let output = String::from_utf8(output)?;
        assert!(output.contains("allowed_roots=legacy-all-approved"));
        Ok(())
    }
}
