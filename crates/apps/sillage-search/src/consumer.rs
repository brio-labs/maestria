use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sillage_daemon::{SearchApiClient, SearchApiOperation};

use super::{ConsumerApiArgs, parse_realm_id};

pub(super) const MAX_CONSUMER_PROFILE_BYTES: u64 = 8_192;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConsumerConnectionProfile {
    pub(super) socket_path: PathBuf,
    pub(super) consumer_realm: String,
    pub(super) credential_file: PathBuf,
}

pub(super) fn create_consumer_profile(
    profile_path: &Path,
    socket_path: PathBuf,
    consumer_realm: String,
    credential_file: PathBuf,
) -> Result<()> {
    let profile = validate_consumer_profile(ConsumerConnectionProfile {
        socket_path,
        consumer_realm,
        credential_file,
    })?;
    let contents =
        serde_json::to_vec_pretty(&profile).context("serialize consumer connection profile")?;
    if contents.len() as u64 + 1 > MAX_CONSUMER_PROFILE_BYTES {
        bail!("consumer connection profile exceeds the maximum size");
    }

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(profile_path).with_context(|| {
        format!(
            "create consumer connection profile {}",
            profile_path.display()
        )
    })?;
    let write_result = file
        .write_all(&contents)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all());
    if let Err(error) = write_result {
        drop(file);
        return match fs::remove_file(profile_path) {
            Ok(()) => Err(error).with_context(|| {
                format!(
                    "write consumer connection profile {}",
                    profile_path.display()
                )
            }),
            Err(remove_error) => Err(error).context(format!(
                "write failed and partial consumer connection profile {} could not be removed: {remove_error}",
                profile_path.display()
            )),
        };
    }
    Ok(())
}

pub(super) fn resolve_consumer_connection(
    args: ConsumerApiArgs,
) -> Result<ConsumerConnectionProfile> {
    match args.profile {
        Some(profile_path) => {
            if args.socket_path.is_some()
                || args.consumer_realm.is_some()
                || args.credential_file.is_some()
            {
                bail!("--profile cannot be combined with direct consumer connection arguments");
            }
            load_consumer_profile(&profile_path)
        }
        None => Ok(ConsumerConnectionProfile {
            socket_path: args
                .socket_path
                .context("missing required --socket-path without --profile")?,
            consumer_realm: args
                .consumer_realm
                .context("missing required --consumer-realm without --profile")?,
            credential_file: args
                .credential_file
                .context("missing required --credential-file without --profile")?,
        }),
    }
}

fn load_consumer_profile(path: &Path) -> Result<ConsumerConnectionProfile> {
    let mut file = File::open(path)
        .with_context(|| format!("open consumer connection profile {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("inspect consumer connection profile {}", path.display()))?;
    if !metadata.is_file() {
        bail!("consumer connection profile must be a regular file");
    }
    if metadata.len() > MAX_CONSUMER_PROFILE_BYTES {
        bail!("consumer connection profile exceeds the maximum size");
    }

    let mut contents = Vec::with_capacity(metadata.len() as usize);
    Read::take(&mut file, MAX_CONSUMER_PROFILE_BYTES + 1)
        .read_to_end(&mut contents)
        .with_context(|| format!("read consumer connection profile {}", path.display()))?;
    if contents.len() as u64 > MAX_CONSUMER_PROFILE_BYTES {
        bail!("consumer connection profile exceeds the maximum size");
    }
    let profile: ConsumerConnectionProfile = serde_json::from_slice(&contents)
        .with_context(|| format!("parse consumer connection profile {}", path.display()))?;
    validate_consumer_profile(profile)
}

fn validate_consumer_profile(
    profile: ConsumerConnectionProfile,
) -> Result<ConsumerConnectionProfile> {
    if profile.socket_path.as_os_str().is_empty() {
        bail!("consumer connection profile socket path must not be empty");
    }
    if profile.credential_file.as_os_str().is_empty() {
        bail!("consumer connection profile credential-file path must not be empty");
    }
    parse_realm_id(profile.consumer_realm.clone()).context("validate consumer profile realm")?;
    Ok(profile)
}

pub(super) async fn run_consumer_request(
    args: ConsumerApiArgs,
    operation: SearchApiOperation,
) -> Result<()> {
    let connection = resolve_consumer_connection(args)?;
    let consumer_realm = parse_realm_id(connection.consumer_realm)?;
    let credential = read_credential_file(&connection.credential_file)?;
    let client = SearchApiClient::consumer(connection.socket_path, consumer_realm, credential)
        .context("construct search API client")?;
    // Keep the client's typed authorization, protocol-version, input, and
    // service-availability errors intact for the CLI caller.
    let response = client.request(operation).await?;
    writeln!(
        std::io::stdout().lock(),
        "{}",
        serde_json::to_string_pretty(&response)?
    )?;
    Ok(())
}

fn read_credential_file(path: &Path) -> Result<String> {
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;

        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        options
            .open(path)
            .with_context(|| format!("open federation credential file {}", path.display()))?
    };
    #[cfg(not(unix))]
    let mut file: File = {
        let _ = path;
        return Err(anyhow!(
            "external federation credential files require Unix permissions"
        ));
    };

    let metadata = file
        .metadata()
        .with_context(|| format!("inspect federation credential file {}", path.display()))?;
    if !metadata.is_file() {
        bail!("federation credential path must be a regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("federation credential file must not be accessible by group or others");
        }
    }
    if metadata.len() > 65 {
        bail!("federation credential file exceeds the expected size");
    }
    let mut contents = String::new();
    Read::take(&mut file, 66)
        .read_to_string(&mut contents)
        .with_context(|| format!("read federation credential file {}", path.display()))?;
    if contents.len() > 65 {
        bail!("federation credential file exceeds the expected size");
    }
    let credential = contents.trim().to_owned();
    if credential.is_empty() {
        bail!("federation credential file is empty");
    }
    Ok(credential)
}

pub(super) fn write_credential_file(path: &Path, credential: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut options = OpenOptions::new();
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
            return match fs::remove_file(path) {
                Ok(()) => Err(error)
                    .with_context(|| format!("write private credential file {}", path.display())),
                Err(remove_error) => Err(error).context(format!(
                    "write failed and partial credential file {} could not be removed: {remove_error}",
                    path.display()
                )),
            };
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, credential);
        bail!("external federation credential files require Unix permissions")
    }
}
