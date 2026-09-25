use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use maestria_daemon::{SearchApiClient, SearchApiOperation};
use maestria_domain::RealmId;

use crate::cli_types::SearchApiCommands;

/// Runs one opt-in search-only request against a provider's local socket.
///
/// # Cancellation
/// Dropping this future closes the client socket and stops waiting for the daemon response.
pub async fn run(command: SearchApiCommands) -> Result<()> {
    let (socket_path, consumer_realm, credential_file, operation) = match command {
        SearchApiCommands::Search {
            socket_path,
            consumer_realm,
            credential_file,
            query,
            limit,
        } => (
            socket_path,
            consumer_realm,
            credential_file,
            SearchApiOperation::Search { query, limit },
        ),
        SearchApiCommands::Status {
            socket_path,
            consumer_realm,
            credential_file,
        } => (
            socket_path,
            consumer_realm,
            credential_file,
            SearchApiOperation::Status,
        ),
        SearchApiCommands::IndexingStatus {
            socket_path,
            consumer_realm,
            credential_file,
        } => (
            socket_path,
            consumer_realm,
            credential_file,
            SearchApiOperation::IndexingStatus,
        ),
        SearchApiCommands::OpenEvidence {
            socket_path,
            consumer_realm,
            credential_file,
            evidence_id,
        } => (
            socket_path,
            consumer_realm,
            credential_file,
            SearchApiOperation::Evidence { evidence_id },
        ),
    };
    let client = consumer_client(socket_path, consumer_realm, credential_file)?;
    let response = client.request(operation).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn consumer_client(
    socket_path: PathBuf,
    consumer_realm: String,
    credential_file: PathBuf,
) -> Result<SearchApiClient> {
    let metadata = fs::symlink_metadata(&credential_file).with_context(|| {
        format!(
            "read federation credential file {}",
            credential_file.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        bail!("federation credential path must be a regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("federation credential file must not be accessible by group or others");
        }
    }
    let credential = fs::read_to_string(&credential_file)
        .with_context(|| format!("read federation credential {}", credential_file.display()))?;
    let credential = credential.trim().to_owned();
    if credential.is_empty() {
        return Err(anyhow!("federation credential file is empty"));
    }
    let consumer_realm = RealmId::try_from(consumer_realm).map_err(|error| anyhow!(error))?;
    SearchApiClient::consumer(socket_path, consumer_realm, credential)
        .context("construct search API client")
}
