#[cfg(unix)]
use std::io::Write;
use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_daemon::{
    ClientOperation, ClientResponse, DaemonClient, RealmGrantAccess, RealmGrantSensitivity,
};
use maestria_domain::RealmId;

use crate::cli_types::{
    CliRealmGrantAccess, CliRealmGrantSensitivity, RealmCommands, RealmGrantCommands,
};
use crate::helpers;

struct GrantPolicy {
    access: CliRealmGrantAccess,
    max_sensitivity: CliRealmGrantSensitivity,
    max_results: usize,
    max_evidence_bytes: usize,
    expires_in_seconds: u64,
}

/// Dispatch one explicit local-realm command.
///
/// # Cancellation
/// Dropping the future stops awaiting any in-flight daemon request. A request already received
/// by a daemon may continue and commit its durable operation.
pub async fn run(command: RealmCommands) -> Result<()> {
    match command {
        RealmCommands::Migrate { instance_dir } => migrate(instance_dir),
        RealmCommands::Identity { instance_dir } => identity(instance_dir),
        RealmCommands::Grant { command } => grant(command).await,
        RealmCommands::Search {
            instance_dir,
            provider_realm,
            query,
            limit,
        } => search(instance_dir, parse_realm_id(provider_realm)?, query, limit).await,
        RealmCommands::OpenEvidence {
            instance_dir,
            provider_realm,
            evidence_id,
        } => open_evidence(instance_dir, parse_realm_id(provider_realm)?, evidence_id).await,
    }
}

fn migrate(instance_dir: PathBuf) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
    let contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    let migrated = InstanceManifest::migrate_v1(&contents, generate_realm_id()?)?;
    fs::write(&layout.manifest_path, migrated.encode())
        .with_context(|| format!("write migrated manifest {}", layout.manifest_path.display()))?;
    println!("realm_id={}", migrated.realm_id.as_str());
    Ok(())
}

fn identity(instance_dir: PathBuf) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let manifest = helpers::load_manifest(&layout)?;
    println!("realm_id={}", manifest.realm_id.as_str());
    Ok(())
}

async fn grant(command: RealmGrantCommands) -> Result<()> {
    match command {
        RealmGrantCommands::Create {
            instance_dir,
            consumer_instance,
            access,
            max_sensitivity,
            max_results,
            max_evidence_bytes,
            expires_in_seconds,
        } => {
            create_grant(
                instance_dir,
                consumer_instance,
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
        RealmGrantCommands::CreateExternal {
            instance_dir,
            consumer_realm,
            credential_file,
            access,
            max_sensitivity,
            max_results,
            max_evidence_bytes,
            expires_in_seconds,
        } => {
            create_external_grant(
                instance_dir,
                parse_realm_id(consumer_realm)?,
                credential_file,
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
        RealmGrantCommands::List { instance_dir } => list_grants(instance_dir).await,
        RealmGrantCommands::Revoke {
            instance_dir,
            grant_token_digest,
        } => revoke_grant(instance_dir, grant_token_digest).await,
    }
}

async fn create_grant(
    provider_instance: PathBuf,
    consumer_instance: PathBuf,
    policy: GrantPolicy,
) -> Result<()> {
    let provider_layout = helpers::validated_instance(provider_instance)?;
    let consumer_layout = helpers::validated_instance(consumer_instance)?;
    let consumer_realm = helpers::load_manifest(&consumer_layout)?.realm_id;
    let provider_socket_path = provider_socket_path(&provider_layout)?;
    let provider_client = DaemonClient::from_instance(&provider_layout)?;
    let created = issue_grant(&provider_client, consumer_realm, policy).await?;
    let grant_digest = created.grant.token_digest.clone();
    let consumer_client = DaemonClient::from_instance(&consumer_layout)?;
    let installed = consumer_client
        .request(ClientOperation::InstallFederationBinding {
            provider_realm: created.grant.provider_realm.clone(),
            provider_socket_path,
            credential: created.credential,
        })
        .await;
    if let Err(error) = installed {
        return Err(error).context(format!(
            "consumer binding installation failed; revoke provider grant {grant_digest} before retrying"
        ));
    }
    print_grant(&created.grant)?;
    Ok(())
}

async fn create_external_grant(
    provider_instance: PathBuf,
    consumer_realm: RealmId,
    credential_file: PathBuf,
    policy: GrantPolicy,
) -> Result<()> {
    let provider_layout = helpers::validated_instance(provider_instance)?;
    let provider_client = DaemonClient::from_instance(&provider_layout)?;
    let created = issue_grant(&provider_client, consumer_realm, policy).await?;
    if let Err(error) = write_credential_file(&credential_file, created.credential.expose()) {
        let grant_digest = created.grant.token_digest.clone();
        return match provider_client
            .request(ClientOperation::RealmGrantRevoke {
                token_digest: grant_digest.clone(),
            })
            .await
        {
            Ok(ClientResponse::RealmGrantList(_)) => Err(error)
                .context("credential file creation failed; the issued grant was revoked"),
            Ok(_) => Err(error).context(format!(
                "credential file creation failed; revoke provider grant {grant_digest} manually"
            )),
            Err(revoke_error) => Err(error).context(format!(
                "credential file creation failed and grant {grant_digest} could not be revoked: {revoke_error}"
            )),
        };
    }
    print_grant(&created.grant)?;
    println!("credential_file={}", credential_file.display());
    Ok(())
}

async fn issue_grant(
    provider_client: &DaemonClient,
    consumer_realm: RealmId,
    policy: GrantPolicy,
) -> Result<maestria_daemon::RealmGrantCreatedResponse> {
    match provider_client
        .request(ClientOperation::RealmGrantCreate {
            consumer_realm,
            access: protocol_access(policy.access),
            max_sensitivity: protocol_sensitivity(policy.max_sensitivity),
            allowed_roots: Vec::new(),
            max_results: policy.max_results,
            max_evidence_bytes: policy.max_evidence_bytes,
            expires_in_seconds: policy.expires_in_seconds,
        })
        .await?
    {
        ClientResponse::RealmGrantCreated(created) => Ok(created),
        _ => bail!("provider returned an unexpected realm-grant creation response"),
    }
}

fn write_credential_file(path: &PathBuf, credential: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = options
            .open(path)
            .with_context(|| format!("create private credential file {}", path.display()))?;
        let write_result = file
            .write_all(credential.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all());
        if let Err(error) = write_result {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(error)
                .with_context(|| format!("write private credential file {}", path.display()));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, credential);
        bail!("external federation credential files require Unix permissions")
    }
}

fn print_grant(grant: &maestria_daemon::RealmGrantResponse) -> Result<()> {
    println!("grant_token_digest={}", grant.token_digest);
    println!("provider_realm={}", grant.provider_realm.as_str());
    println!("consumer_realm={}", grant.consumer_realm.as_str());
    println!("access={}", display_access(grant.access));
    println!(
        "max_sensitivity={}",
        display_sensitivity(grant.max_sensitivity)
    );
    println!("max_results={}", grant.max_results);
    println!("max_evidence_bytes={}", grant.max_evidence_bytes);
    println!("expires_at_unix_seconds={}", grant.expires_at_unix_seconds);
    println!("state={}", grant.state);
    println!(
        "allowed_roots={}",
        display_allowed_roots(&grant.allowed_roots)?
    );
    Ok(())
}

fn display_allowed_roots(allowed_roots: &Option<Vec<String>>) -> Result<String> {
    match allowed_roots {
        Some(roots) => Ok(serde_json::to_string(roots)?),
        None => Ok("legacy-all-approved".to_string()),
    }
}

async fn list_grants(instance_dir: PathBuf) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let response = DaemonClient::from_instance(&layout)?
        .request(ClientOperation::RealmGrantList)
        .await?;
    let ClientResponse::RealmGrantList(list) = response else {
        bail!("provider returned an unexpected realm-grant list response");
    };
    for grant in list.grants {
        println!(
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
        );
        if grant.expires_at_unix_seconds == 1 {
            println!(
                "legacy_grant_reissue_required=true grant_token_digest={}",
                grant.token_digest
            );
        }
    }
    Ok(())
}

async fn revoke_grant(instance_dir: PathBuf, grant_token_digest: String) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let response = DaemonClient::from_instance(&layout)?
        .request(ClientOperation::RealmGrantRevoke {
            token_digest: grant_token_digest.clone(),
        })
        .await?;
    let ClientResponse::RealmGrantList(_) = response else {
        bail!("provider returned an unexpected realm-grant revocation response");
    };
    println!("grant_token_digest={grant_token_digest}");
    println!("state=revoked");
    Ok(())
}

async fn search(
    consumer_instance: PathBuf,
    provider_realm: RealmId,
    query: String,
    limit: usize,
) -> Result<()> {
    let layout = helpers::validated_instance(consumer_instance)?;
    let response = DaemonClient::from_instance(&layout)?
        .request(ClientOperation::FederationSearch {
            provider_realm: provider_realm.clone(),
            query,
            limit,
        })
        .await?;
    let ClientResponse::FederationSearch(response) = response else {
        bail!("consumer daemon returned an unexpected federation search response");
    };
    if response.provider_realm != provider_realm {
        return Err(anyhow!(
            "consumer daemon returned a response from another provider realm"
        ));
    }
    println!("provider_realm={}", response.provider_realm.as_str());
    println!("graph_degraded={}", response.graph_degraded);
    println!("status={}", response.search.status);
    for (rank, evidence) in response.search.evidence.iter().enumerate() {
        println!(
            "rank={} evidence_id={} source={} range={}-{}",
            rank + 1,
            evidence.evidence_id,
            evidence.source,
            evidence.range_start,
            evidence.range_end,
        );
    }
    Ok(())
}

async fn open_evidence(
    consumer_instance: PathBuf,
    provider_realm: RealmId,
    evidence_id: u64,
) -> Result<()> {
    let layout = helpers::validated_instance(consumer_instance)?;
    let response = DaemonClient::from_instance(&layout)?
        .request(ClientOperation::FederationEvidence {
            provider_realm: provider_realm.clone(),
            evidence_id,
        })
        .await?;
    let ClientResponse::FederationEvidence(response) = response else {
        bail!("consumer daemon returned an unexpected federation evidence response");
    };
    if response.provider_realm != provider_realm {
        return Err(anyhow!(
            "consumer daemon returned a response from another provider realm"
        ));
    }
    println!("provider_realm={}", response.provider_realm.as_str());
    println!("evidence_id={}", response.evidence.evidence_id);
    println!("source={:?}", response.evidence.source);
    println!("excerpt={}", response.evidence.excerpt);
    Ok(())
}

fn parse_realm_id(value: String) -> Result<RealmId> {
    RealmId::try_from(value).map_err(|error| anyhow!(error))
}

fn generate_realm_id() -> Result<RealmId> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| anyhow!("generate realm identity: {error}"))?;
    RealmId::try_from(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
        .map_err(|error| anyhow!(error))
}

fn protocol_access(value: CliRealmGrantAccess) -> RealmGrantAccess {
    match value {
        CliRealmGrantAccess::SearchOnly => RealmGrantAccess::SearchOnly,
        CliRealmGrantAccess::SearchAndOpenEvidence => RealmGrantAccess::SearchAndOpenEvidence,
    }
}

fn provider_socket_path(layout: &InstanceLayout) -> Result<String> {
    let root = fs::canonicalize(&layout.root)
        .with_context(|| format!("canonicalize provider instance {}", layout.root.display()))?;
    let socket = root.join("system").join("daemon.sock");
    if !socket.exists() {
        bail!(
            "provider daemon socket is unavailable at {}; start the provider daemon first",
            socket.display()
        );
    }
    Ok(socket.display().to_string())
}

fn protocol_sensitivity(value: CliRealmGrantSensitivity) -> RealmGrantSensitivity {
    match value {
        CliRealmGrantSensitivity::Public => RealmGrantSensitivity::Public,
        CliRealmGrantSensitivity::Internal => RealmGrantSensitivity::Internal,
        CliRealmGrantSensitivity::Confidential => RealmGrantSensitivity::Confidential,
        CliRealmGrantSensitivity::Restricted => RealmGrantSensitivity::Restricted,
    }
}

fn display_access(value: RealmGrantAccess) -> &'static str {
    match value {
        RealmGrantAccess::SearchOnly => "search-only",
        RealmGrantAccess::SearchAndOpenEvidence => "search-and-open-evidence",
    }
}

fn display_sensitivity(value: RealmGrantSensitivity) -> &'static str {
    match value {
        RealmGrantSensitivity::Public => "public",
        RealmGrantSensitivity::Internal => "internal",
        RealmGrantSensitivity::Confidential => "confidential",
        RealmGrantSensitivity::Restricted => "restricted",
    }
}
