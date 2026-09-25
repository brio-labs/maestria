#[cfg(not(unix))]
use std::fs::File;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

#[cfg(not(unix))]
use anyhow::anyhow;
use anyhow::{Context, Result, bail};
use maestria_daemon::{SearchApiClient, SearchApiOperation};

use super::{ConsumerApiArgs, parse_realm_id};
pub(super) async fn run_consumer_request(
    args: ConsumerApiArgs,
    operation: SearchApiOperation,
) -> Result<()> {
    let consumer_realm = parse_realm_id(args.consumer_realm)?;
    let credential = read_credential_file(&args.credential_file)?;
    let client = SearchApiClient::consumer(args.socket_path, consumer_realm, credential)
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
