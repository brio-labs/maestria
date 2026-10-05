use super::*;
pub(super) fn acquire_profile_lock(profile_root: &Path) -> Result<File, String> {
    let path = profile_root.join("system/launcher.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|_| "Could not access the private managed search lock.".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "Could not secure the private managed search lock.".to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            return Err(
                "Another launcher process is managing this private search profile.".to_string(),
            );
        }
    }
    Ok(file)
}
pub(super) fn read_marker(profile_root: &Path) -> Result<Option<ProfileMarker>, String> {
    let path = profile_root.join(MARKER_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not inspect launcher search ownership provenance.".to_string()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 32 * 1024 {
        return Err("The launcher search ownership marker is invalid.".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("The launcher search ownership marker is not private.".to_string());
        }
    }
    let contents = fs::read(&path)
        .map_err(|_| "Could not read launcher search ownership provenance.".to_string())?;
    let marker: ProfileMarker = serde_json::from_slice(&contents)
        .map_err(|_| "The launcher search ownership marker is malformed.".to_string())?;
    if marker.schema_version != 1
        || !is_hex_digest(&marker.profile_identity)
        || !is_hex_digest(&marker.consumer_realm)
    {
        return Err("The launcher search ownership marker is invalid.".to_string());
    }
    Ok(Some(marker))
}
pub(super) fn unmarked_profile_has_state(profile_root: &Path) -> Result<bool, String> {
    for relative in ["manifest.txt", "system/daemon.token", "system/sillage.db"] {
        match fs::symlink_metadata(profile_root.join(relative)) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not inspect the existing search profile.".to_string()),
        }
    }
    let entries = fs::read_dir(profile_root)
        .map_err(|_| "Could not inspect the existing search profile.".to_string())?;
    for entry in entries {
        let entry =
            entry.map_err(|_| "Could not inspect the existing search profile.".to_string())?;
        let name = entry.file_name();
        if !name
            .to_str()
            .is_some_and(|name| ["blobs", "indexes", "workspace", "system"].contains(&name))
        {
            return Ok(true);
        }
    }
    for (relative, allowed) in [
        ("blobs", &["sha256"][..]),
        ("indexes", &["full-text", "vector", "graph"][..]),
        ("workspace", &["active_tasks"][..]),
        (
            "system",
            &[
                "launcher.lock",
                "config",
                "policies",
                "logs",
                "evidence_registry",
                "event_log",
            ][..],
        ),
    ] {
        let entries = match fs::read_dir(profile_root.join(relative)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return Err("Could not inspect the existing search profile.".to_string()),
        };
        for entry in entries {
            let entry =
                entry.map_err(|_| "Could not inspect the existing search profile.".to_string())?;
            let name = entry.file_name();
            if !name.to_str().is_some_and(|name| allowed.contains(&name)) {
                return Ok(true);
            }
        }
    }
    for relative in [
        "blobs/sha256",
        "indexes/full-text",
        "indexes/vector",
        "indexes/graph",
        "workspace/active_tasks",
        "system/config",
        "system/policies",
        "system/logs",
        "system/evidence_registry",
        "system/event_log",
    ] {
        let entries = match fs::read_dir(profile_root.join(relative)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return Err("Could not inspect the existing search profile.".to_string()),
        };
        if entries.count() != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}
pub(super) fn validate_marker(
    marker: &ProfileMarker,
    managed: Option<&ManagedSearchConfig>,
) -> Result<(), String> {
    if marker.schema_version != 1
        || !is_hex_digest(&marker.profile_identity)
        || !is_hex_digest(&marker.consumer_realm)
        || marker.roots.iter().any(|root| !root.is_absolute())
        || marker.grants.iter().any(|grant| {
            !is_hex_digest(&grant.token_digest)
                || !grant.root.is_absolute()
                || !grant.credential_file.is_absolute()
                || grant.expires_at_unix_seconds == 0
        })
    {
        return Err("The launcher search ownership marker is invalid.".to_string());
    }
    if let Some(managed) = managed
        && marker.profile_identity != managed.profile_identity
    {
        return Err(
            "The saved profile identity does not match launcher ownership provenance.".to_string(),
        );
    }
    Ok(())
}
pub(super) fn validate_marker_credentials(
    marker: &ProfileMarker,
    profile_root: &Path,
) -> Result<(), String> {
    if marker
        .grants
        .iter()
        .any(|grant| !valid_credential_path(profile_root, &grant.credential_file))
        || marker
            .pending_credential_file
            .as_ref()
            .is_some_and(|file| !valid_credential_path(profile_root, file))
    {
        return Err(
            "The launcher search marker contains an out-of-profile credential reference."
                .to_string(),
        );
    }
    Ok(())
}
pub(super) fn valid_credential_path(profile_root: &Path, file: &Path) -> bool {
    let system_dir = profile_root.join("system");
    if file.parent() != Some(system_dir.as_path()) {
        return false;
    }
    let Some(name) = file.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(random) = name
        .strip_prefix("launcher-search-")
        .and_then(|name| name.strip_suffix(".credential"))
    else {
        return false;
    };
    random.len() == 16 && random.bytes().all(|byte| byte.is_ascii_hexdigit())
}
pub(super) fn write_marker(profile_root: &Path, marker: &ProfileMarker) -> Result<(), String> {
    let path = profile_root.join(MARKER_FILE);
    let temporary = profile_root.join(format!(".{MARKER_FILE}.tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec(marker)
        .map_err(|_| "Could not encode launcher search ownership provenance.".to_string())?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(&temporary) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_file(&temporary)
                .map_err(|_| "Could not replace private search provenance.".to_string())?;
            let mut retry = OpenOptions::new();
            retry.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                retry.mode(0o600);
            }
            retry
                .open(&temporary)
                .map_err(|_| "Could not write private search provenance.".to_string())?
        }
        Err(_) => return Err("Could not write private search provenance.".to_string()),
    };
    if file
        .write_all(&bytes)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        let _ = fs::remove_file(&temporary);
        return Err("Could not persist private search provenance.".to_string());
    }
    drop(file);
    fs::rename(&temporary, &path)
        .map_err(|_| "Could not replace private search provenance.".to_string())
}
pub(super) fn remove_private_file(profile_root: &Path, file: &Path) -> Result<(), String> {
    if !valid_credential_path(profile_root, file) {
        return Err(
            "Refusing to remove a credential outside the private managed profile.".to_string(),
        );
    }
    match fs::symlink_metadata(file) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("The managed search credential is not a regular private file.".to_string())
        }
        Ok(_) => fs::remove_file(file)
            .map_err(|_| "Could not remove the private managed search credential.".to_string()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not inspect the private managed search credential.".to_string()),
    }
}
pub(super) fn verify_private_credential(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "Sillage search did not create the private grant credential.".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 65 {
        return Err("The managed search credential is not a valid private file.".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("The managed search credential permissions are not private.".to_string());
        }
    }
    Ok(())
}
pub(super) fn random_hex(byte_count: usize) -> Result<String, String> {
    let mut bytes = vec![0_u8; byte_count];
    let mut random = File::open("/dev/urandom").map_err(|_| {
        "The operating system cryptographic random source is unavailable.".to_string()
    })?;
    random
        .read_exact(&mut bytes)
        .map_err(|_| "The operating system cryptographic random source failed.".to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
pub(super) fn is_hex_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
pub(super) fn expected_token_digest(path: &Path) -> Result<String, String> {
    verify_private_credential(path)?;
    let token = fs::read_to_string(path)
        .map_err(|_| "Could not read private managed grant metadata.".to_string())?;
    let token = token.trim();
    if token.is_empty() || token.len() > 65 || !token.is_ascii() {
        return Err("The private managed grant metadata is invalid.".to_string());
    }
    Ok(GrantTokenDigest::derive(token.as_bytes()).to_string())
}
pub(super) fn sanitize_error(
    error: &str,
    service: Option<&SearchServiceConfig>,
    managed: Option<&ManagedSearchConfig>,
) -> String {
    let mut safe = error.to_string();
    if let Some(service) = service {
        safe = safe.replace(&service.consumer_realm, "[private realm]");
        safe = safe.replace(
            &service.credential_file.to_string_lossy().to_string(),
            "[private credential]",
        );
        if let Ok(token) = fs::read_to_string(&service.credential_file) {
            let token = token.trim();
            if !token.is_empty() {
                safe = safe.replace(token, "[private credential]");
            }
        }
    }
    if let Some(managed) = managed {
        safe = safe.replace(&managed.profile_identity, "[private profile]");
        safe = safe.replace(&managed.grant_token_digest, "[private grant]");
    }
    bounded(&safe, 512)
}
pub(super) fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}
pub(super) fn same_path(displayed: &str, expected: &Path) -> bool {
    expected.to_str() == Some(displayed)
}
