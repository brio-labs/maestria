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
        } => {
            create_grant(
                instance_dir,
                consumer_instance,
                access,
                max_sensitivity,
                max_results,
                max_evidence_bytes,
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
    access: CliRealmGrantAccess,
    max_sensitivity: CliRealmGrantSensitivity,
    max_results: usize,
    max_evidence_bytes: usize,
) -> Result<()> {
    let provider_layout = helpers::validated_instance(provider_instance)?;
    let consumer_layout = helpers::validated_instance(consumer_instance)?;
    let consumer_realm = helpers::load_manifest(&consumer_layout)?.realm_id;
    let provider_socket_path = provider_socket_path(&provider_layout)?;
    let provider_client = DaemonClient::from_instance(&provider_layout)?;
    let response = provider_client
        .request(ClientOperation::RealmGrantCreate {
            consumer_realm,
            access: protocol_access(access),
            max_sensitivity: protocol_sensitivity(max_sensitivity),
            max_results,
            max_evidence_bytes,
        })
        .await?;
    let ClientResponse::RealmGrantCreated(created) = response else {
        bail!("provider returned an unexpected realm-grant creation response");
    };
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
    println!("grant_token_digest={grant_digest}");
    println!("provider_realm={}", created.grant.provider_realm.as_str());
    println!("consumer_realm={}", created.grant.consumer_realm.as_str());
    println!("access={}", display_access(created.grant.access));
    println!(
        "max_sensitivity={}",
        display_sensitivity(created.grant.max_sensitivity)
    );
    println!("max_results={}", created.grant.max_results);
    println!("max_evidence_bytes={}", created.grant.max_evidence_bytes);
    Ok(())
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
            "grant_token_digest={} provider_realm={} consumer_realm={} access={} max_sensitivity={} max_results={} max_evidence_bytes={} state={}",
            grant.token_digest,
            grant.provider_realm.as_str(),
            grant.consumer_realm.as_str(),
            display_access(grant.access),
            display_sensitivity(grant.max_sensitivity),
            grant.max_results,
            grant.max_evidence_bytes,
            grant.state,
        );
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
    getrandom::getrandom(&mut bytes)
        .map_err(|error| anyhow!("generate realm identity: {error}"))?;
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
