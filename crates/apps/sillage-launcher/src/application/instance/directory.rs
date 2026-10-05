use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

use super::permission_denied;

pub(super) fn runtime_dir() -> Result<PathBuf, io::Error> {
    let uid = unsafe { libc::geteuid() };
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let runtime = PathBuf::from(runtime);
        if runtime.is_absolute()
            && fs::symlink_metadata(&runtime).is_ok_and(|metadata| {
                metadata.file_type().is_dir()
                    && metadata.uid() == uid
                    && metadata.permissions().mode() & 0o077 == 0
            })
        {
            return Ok(runtime);
        }
    }

    let fallback = std::env::temp_dir().join(format!("sillage-launcher-{uid}"));
    match fs::create_dir(&fallback) {
        Ok(()) => {
            let metadata = fs::symlink_metadata(&fallback)?;
            validate_runtime_directory(&metadata, uid)?;
            if metadata.permissions().mode() & 0o777 != 0o700 {
                fs::set_permissions(&fallback, fs::Permissions::from_mode(0o700))?;
            }
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&fallback)?;
            validate_runtime_directory(&metadata, uid)?;
        }
        Err(error) => return Err(error),
    }
    Ok(fallback)
}

fn validate_runtime_directory(metadata: &fs::Metadata, uid: u32) -> io::Result<()> {
    if !metadata.file_type().is_dir()
        || metadata.uid() != uid
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(permission_denied(
            "launcher runtime directory is not private to the current user",
        ));
    }
    Ok(())
}
