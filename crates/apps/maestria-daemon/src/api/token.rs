use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use maestria_core::InstanceLayout;

const TOKEN_BYTES: usize = 32;

pub(crate) fn socket_path(layout: &InstanceLayout) -> PathBuf {
    layout.system_dir.join("daemon.sock")
}

pub(crate) fn token_path(layout: &InstanceLayout) -> PathBuf {
    layout.system_dir.join("daemon.token")
}

pub(crate) fn load_or_create_token(path: &Path) -> Result<String> {
    if let Ok(contents) = fs::read_to_string(path) {
        let token = contents.trim().to_string();
        validate_token(&token)?;
        set_private_permissions(path)?;
        return Ok(token);
    }
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| anyhow!("generate daemon credential: {error}"))?;
    let token = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::read_to_string(path)?;
            let token = existing.trim().to_string();
            validate_token(&token)?;
            set_private_permissions(path)?;
            return Ok(token);
        }
        Err(error) => return Err(error.into()),
    };
    file.write_all(token.as_bytes())?;
    file.write_all(b"\n")?;
    set_private_permissions(path)?;
    Ok(token)
}

pub(crate) fn validate_token(token: &str) -> Result<()> {
    if token.len() != TOKEN_BYTES * 2 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!("daemon token has invalid format"));
    }
    Ok(())
}

pub(crate) fn remove_stale_socket(path: &Path) -> Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Err(anyhow!(
            "daemon socket path is occupied by a regular file: {}",
            path.display()
        )),
        Ok(_) => {
            fs::remove_file(path)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn set_private_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub(crate) fn set_private_directory_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_validation_rejects_short_or_non_hex_credentials()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(validate_token("short").is_err());
        assert!(validate_token(&"z".repeat(TOKEN_BYTES * 2)).is_err());
        let valid = "a".repeat(TOKEN_BYTES * 2);
        validate_token(&valid)?;
        Ok(())
    }
}
