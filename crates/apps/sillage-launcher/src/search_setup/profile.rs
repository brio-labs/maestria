use super::*;
use sillage_core::InstanceManifest;

pub(super) async fn initialize_profile_if_new(
    setup: &SearchSetup,
    profile_root: &Path,
    startup_root: &Path,
    previously_consented_root: Option<&Path>,
    program: &Path,
    generation: u64,
) -> Result<(), String> {
    let manifest = profile_root.join("manifest.txt");
    match fs::symlink_metadata(&manifest) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(
                    "The private search profile manifest is not a regular file.".to_string()
                );
            }
            if let Some(root) = previously_consented_root {
                validate_manifest_roots(profile_root, Some(root))?;
            } else {
                replace_manifest_roots(profile_root, &[])?;
                validate_manifest_roots(profile_root, None)?;
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            setup
                .cli(
                    generation,
                    program,
                    "init",
                    vec![
                        "init".into(),
                        "--instance-dir".into(),
                        profile_root.as_os_str().to_os_string(),
                        "--read-root".into(),
                        path_argument(startup_root)?,
                    ],
                )
                .await?;
            ensure_private_profile_dirs(profile_root)?;
            validate_manifest_roots(profile_root, Some(startup_root))?;
        }
        Err(_) => return Err("Could not inspect the private search profile manifest.".to_string()),
    }
    Ok(())
}

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

pub(super) fn validate_manifest_roots(
    profile_root: &Path,
    expected_root: Option<&Path>,
) -> Result<(), String> {
    if let Some(root) = expected_root
        && canonical_read_root(root)? != root
    {
        return Err(
            "The approved folder changed since selection; choose the folder again.".to_string(),
        );
    }
    let metadata = read_manifest_metadata(profile_root)?;
    let expected = match expected_root {
        Some(root) => metadata.read_roots.len() == 1 && metadata.read_roots[0] == root,
        None => metadata.read_roots.is_empty(),
    };
    if !expected {
        return Err(
            "The search profile is not restricted to the approved folder; choose the folder again."
                .to_string(),
        );
    }
    Ok(())
}

fn read_manifest_metadata(profile_root: &Path) -> Result<InstanceManifest, String> {
    let path = profile_root.join("manifest.txt");
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| "Could not inspect the private search profile manifest.".to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_MANIFEST_BYTES
    {
        return Err("The private search profile manifest is invalid.".to_string());
    }
    let contents = fs::read_to_string(path)
        .map_err(|_| "Could not read the private search profile manifest.".to_string())?;
    InstanceManifest::decode(&contents)
        .map_err(|_| "The private search profile manifest is malformed.".to_string())
}

fn replace_manifest_roots(profile_root: &Path, roots: &[PathBuf]) -> Result<(), String> {
    let mut manifest = read_manifest_metadata(profile_root)?;
    manifest.read_roots = roots.to_vec();
    write_private_manifest(
        &profile_root.join("manifest.txt"),
        manifest.encode().as_bytes(),
    )
}

fn write_private_manifest(path: &Path, contents: &[u8]) -> Result<(), String> {
    let temporary = path.with_file_name(format!(".manifest-{}.tmp", random_hex(8)?));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
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
    let mut file = options
        .open(&temporary)
        .map_err(|_| "Could not create private search manifest metadata.".to_string())?;
    if file
        .write_all(contents)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        let _ = fs::remove_file(&temporary);
        return Err("Could not persist private search manifest metadata.".to_string());
    }
    drop(file);
    fs::rename(&temporary, path)
        .map_err(|_| "Could not replace private search manifest metadata.".to_string())
}
pub(super) fn managed_profile_root() -> Result<PathBuf, String> {
    let data_home = match std::env::var_os("XDG_DATA_HOME") {
        Some(path) => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(
                    "XDG_DATA_HOME must be an absolute path to enable managed search.".to_string(),
                );
            }
            path
        }
        None => std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|home| home.join(".local/share"))
            .ok_or_else(|| {
                "A private XDG data directory or absolute HOME is required for managed search."
                    .to_string()
            })?,
    };
    let mut profile = data_home;
    for component in PROFILE_COMPONENTS {
        profile.push(component);
    }
    if profile.to_str().is_none() {
        return Err("The managed search data directory must be a valid local path.".to_string());
    }
    Ok(profile)
}
pub(super) fn resolve_search_binary() -> Result<PathBuf, String> {
    let name = "sillage-search";
    let installed = Path::new("/usr/bin").join(name);
    if installed.is_file() {
        return installed.canonicalize().map_err(|_| {
            "The installed Sillage search component could not be resolved.".to_string()
        });
    }
    let sibling = std::env::current_exe()
        .map_err(|_| "Could not locate the launcher executable.".to_string())?
        .with_file_name(name);
    if sibling.is_file() {
        return sibling.canonicalize().map_err(|_| {
            "The bundled Sillage search component could not be resolved.".to_string()
        });
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate.canonicalize().map_err(|_| {
                    "The Sillage search component on PATH could not be resolved.".to_string()
                });
            }
        }
    }
    Err("The standalone `sillage-search` component is missing. Install the Sillage search component; the launcher will not download or install it.".to_string())
}
pub(super) fn canonical_read_root(root: &Path) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err("Choose an absolute local folder for document search.".to_string());
    }
    let metadata = fs::symlink_metadata(root).map_err(|_| {
        "The approved folder is unavailable; choose an existing local folder.".to_string()
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(
            "The approved search location must be a real folder, not a symbolic link.".to_string(),
        );
    }
    let canonical = root
        .canonicalize()
        .map_err(|_| "The approved folder could not be resolved.".to_string())?;
    if canonical.to_str().is_none() {
        return Err(
            "The approved folder path must be valid Unicode for Sillage search.".to_string(),
        );
    }
    Ok(canonical)
}

pub(super) fn path_argument(path: &Path) -> Result<std::ffi::OsString, String> {
    if path.to_str().is_none() {
        return Err(
            "The approved folder path must be valid Unicode for Sillage search.".to_string(),
        );
    }
    Ok(path.as_os_str().to_os_string())
}

pub(super) fn validate_socket_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        if bytes.len() >= sun_path_capacity() {
            return Err("The XDG data directory path is too long for a local search socket. Choose a shorter XDG_DATA_HOME or HOME path.".to_string());
        }
        if bytes.contains(&0) {
            return Err(
                "The managed search socket path contains an invalid character.".to_string(),
            );
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err("Managed read-only search is currently supported only on Linux.".to_string())
    }
}

#[cfg(target_os = "linux")]
pub(super) fn sun_path_capacity() -> usize {
    std::mem::size_of::<libc::sockaddr_un>() - std::mem::offset_of!(libc::sockaddr_un, sun_path)
}
pub(super) fn ensure_profile_directory(profile_root: &Path) -> Result<bool, String> {
    let parent = profile_root
        .parent()
        .ok_or_else(|| "The managed search profile has no parent directory.".to_string())?;
    ensure_directory_chain(parent)?;
    match create_private_directory(profile_root) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            ensure_real_directory(profile_root)?;
            Ok(false)
        }
        Err(_) => Err("Could not create a private managed search directory.".to_string()),
    }
}

pub(super) fn ensure_private_profile_dirs(profile_root: &Path) -> Result<(), String> {
    ensure_profile_directory(profile_root)?;
    secure_directory(profile_root)?;
    for relative in [
        "blobs",
        "blobs/sha256",
        "indexes",
        "indexes/full-text",
        "indexes/vector",
        "indexes/graph",
        "workspace",
        "workspace/active_tasks",
        "system",
        "system/config",
        "system/policies",
        "system/logs",
        "system/evidence_registry",
        "system/event_log",
    ] {
        let path = profile_root.join(relative);
        ensure_directory_chain(&path)?;
        secure_directory(&path)?;
    }
    Ok(())
}

fn ensure_directory_chain(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(
                    "A managed search directory is not a private real directory.".to_string(),
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match create_private_directory(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        ensure_real_directory(&current)?;
                    }
                    Err(_) => {
                        return Err(
                            "Could not create a private managed search directory.".to_string()
                        );
                    }
                }
            }
            Err(_) => return Err("Could not inspect a managed search directory.".to_string()),
        }
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn ensure_real_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "Could not inspect a managed search directory.".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("A managed search directory is not a private real directory.".to_string());
    }
    Ok(())
}

fn secure_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Could not secure a managed search directory.".to_string())?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
