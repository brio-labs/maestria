use super::*;
#[cfg(all(test, unix))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::session_search) enum PurgeTestBoundary {
    GenerationMarkerUnlink,
    GenerationDirectoryRemoval,
    SourceMarkerUnlink,
    SourceDirectoryRemoval,
}

#[cfg(all(test, unix))]
pub(in crate::session_search) enum PurgeTestAction {
    Fail,
    CreateUnknown {
        name: &'static str,
        contents: &'static [u8],
    },
}

#[cfg(all(test, unix))]
struct PurgeTestTarget {
    boundary: PurgeTestBoundary,
    identity: (u64, u64),
    action: PurgeTestAction,
}

#[cfg(all(test, unix))]
static PURGE_TEST_ACTION: std::sync::Mutex<Option<PurgeTestTarget>> = std::sync::Mutex::new(None);

#[cfg(all(test, unix))]
pub(in crate::session_search) fn arm_purge_test_action(
    boundary: PurgeTestBoundary,
    directory: &Path,
    action: PurgeTestAction,
) -> Result<(u64, u64)> {
    let identity = purge_test_directory_identity(directory)?;
    let mut pending = PURGE_TEST_ACTION
        .lock()
        .map_err(|error| anyhow!("purge test-action lock failed: {error}"))?;
    if pending.is_some() {
        bail!("a purge test action is already armed");
    }
    *pending = Some(PurgeTestTarget {
        boundary,
        identity,
        action,
    });
    Ok(identity)
}

#[cfg(all(test, unix))]
pub(in crate::session_search) fn clear_purge_test_action(
    boundary: PurgeTestBoundary,
    identity: (u64, u64),
) -> Result<bool> {
    let mut pending = PURGE_TEST_ACTION
        .lock()
        .map_err(|error| anyhow!("purge test-action lock failed: {error}"))?;
    let remains_armed = pending
        .as_ref()
        .is_some_and(|armed| armed.boundary == boundary && armed.identity == identity);
    if remains_armed {
        pending.take();
    }
    Ok(remains_armed)
}

#[cfg(all(test, unix))]
fn purge_test_directory_identity(path: &Path) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("cannot inspect purge test directory {path:?}"))?;
    if !metadata.is_dir() {
        bail!("purge test target is not a directory");
    }
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(all(test, unix))]
fn run_purge_test_action(boundary: PurgeTestBoundary, directory: &File) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        bail!("purge fault boundary is not an open directory");
    }
    let identity = (metadata.dev(), metadata.ino());
    let action = {
        let mut pending = PURGE_TEST_ACTION
            .lock()
            .map_err(|error| anyhow!("purge test-action lock failed: {error}"))?;
        if pending
            .as_ref()
            .is_some_and(|armed| armed.boundary == boundary && armed.identity == identity)
        {
            pending.take().map(|armed| armed.action)
        } else {
            None
        }
    };
    match action {
        Some(PurgeTestAction::Fail) => {
            Err(anyhow!("injected purge filesystem failure at {boundary:?}"))
        }
        Some(PurgeTestAction::CreateUnknown { name, contents }) => {
            let name = c_name(OsStr::new(name))?;
            let mut file = open_file_at(directory, &name, true)?;
            set_private_file_mode(&file)?;
            verify_private_file(&file)?;
            file.write_all(contents)?;
            file.sync_all()?;
            directory.sync_all()?;
            Ok(())
        }
        None => Ok(()),
    }
}

pub(in crate::session_search) fn purge_source(root: &Path, source_path: &Path) -> Result<()> {
    if !root.is_absolute()
        || root.as_os_str().is_empty()
        || root
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        bail!("retention store root must be absolute and normalized");
    }
    let Some(root) = open_store_root(root, false)? else {
        return Ok(());
    };
    let Some(namespace) =
        open_owned_child(&root, ".sillage-session-store-v1", ".owner", STORE_MARKER)?
    else {
        return Ok(());
    };
    let Some(sources) = open_owned_child(
        &namespace,
        "sources",
        ".owner",
        "sillage-source-namespaces-v1\n",
    )?
    else {
        return Ok(());
    };
    let source_key = hash_key(source_path.as_os_str().as_encoded_bytes());
    let source_name = c_name(OsStr::new(&source_key))?;
    let source_marker = format!("{SOURCE_MARKER_PREFIX}{source_key}\n");
    let Some(source) = open_owned_child(&sources, &source_key, ".owner", &source_marker)? else {
        return Ok(());
    };
    validate_source_entries(&source)?;
    let generations_name = c_name(OsStr::new("generations"))?;
    let generations = open_owned_child(
        &source,
        "generations",
        ".owner",
        "sillage-source-generations-v1\n",
    )?;
    if let Some(generations) = generations {
        validate_generation_entries(&generations)?;
        visit_directory_entries(&generations, |name| {
            if name == OsStr::new(".owner") {
                return Ok(());
            }
            if !is_generation_file(name) && !is_staging_file(name) {
                bail!("refusing to purge an unknown item in the source generation namespace");
            }
            let name_c = c_name(name)?;
            let file = open_file_at(&generations, &name_c, false)?;
            verify_private_file(&file)?;
            if !file.metadata()?.is_file() {
                bail!("refusing to purge a non-file documentary generation");
            }
            drop(file);
            unlink_at(&generations, &name_c, false)
                .with_context(|| format!("cannot remove owned generation {name:?}"))?;
            Ok(())
        })?;
        ensure_only_owner_marker(&generations)?;
        remove_owner_marker(&generations, ".owner", "sillage-source-generations-v1\n")
            .context("cannot remove owner marker from empty generations directory")?;
        remove_owned_directory_with_marker(
            &source,
            &generations_name,
            &generations,
            "sillage-source-generations-v1\n",
            "cannot remove empty owned generations directory",
            |directory| {
                #[cfg(all(test, unix))]
                {
                    run_purge_test_action(PurgeTestBoundary::GenerationDirectoryRemoval, directory)
                }
                #[cfg(not(all(test, unix)))]
                {
                    let _ = directory;
                    Ok(())
                }
            },
        )?;
    }
    ensure_only_owner_marker(&source)?;
    remove_owner_marker(&source, ".owner", &source_marker)?;
    remove_owned_directory_with_marker(
        &sources,
        &source_name,
        &source,
        &source_marker,
        "cannot remove empty owned source namespace",
        |directory| {
            #[cfg(all(test, unix))]
            {
                run_purge_test_action(PurgeTestBoundary::SourceDirectoryRemoval, directory)
            }
            #[cfg(not(all(test, unix)))]
            {
                let _ = directory;
                Ok(())
            }
        },
    )?;
    Ok(())
}

fn validate_generation_entries(directory: &File) -> Result<()> {
    visit_directory_entries(directory, |name| {
        if name == OsStr::new(".owner") {
            return Ok(());
        }
        if !is_generation_file(name) && !is_staging_file(name) {
            bail!("refusing to purge an unknown item in the source generation namespace");
        }
        let name_c = c_name(name)?;
        let file = open_file_at(directory, &name_c, false)?;
        verify_private_file(&file)?;
        if !file.metadata()?.is_file() {
            bail!("refusing to purge a non-file documentary generation");
        }
        Ok(())
    })
}

fn validate_source_entries(directory: &File) -> Result<()> {
    visit_directory_entries(directory, |name| {
        if name != OsStr::new(".owner") && name != OsStr::new("generations") {
            bail!("refusing to purge an unknown item in a selected source namespace");
        }
        Ok(())
    })
}

fn ensure_only_owner_marker(directory: &File) -> Result<()> {
    visit_directory_entries(directory, |name| {
        if name != OsStr::new(".owner") {
            bail!("refusing to purge an unknown item in a selected source namespace");
        }
        Ok(())
    })
}

fn remove_owner_marker(directory: &File, marker: &str, contents: &str) -> Result<()> {
    verify_marker(directory, marker, contents)?;
    #[cfg(all(test, unix))]
    run_purge_test_action(
        if contents == "sillage-source-generations-v1\n" {
            PurgeTestBoundary::GenerationMarkerUnlink
        } else {
            PurgeTestBoundary::SourceMarkerUnlink
        },
        directory,
    )?;
    unlink_at(directory, &c_name(OsStr::new(marker))?, false)?;
    Ok(())
}

fn remove_owned_directory_with_marker(
    parent: &File,
    name: &CString,
    directory: &File,
    marker_contents: &str,
    operation: &'static str,
    before_remove: impl FnOnce(&File) -> Result<()>,
) -> Result<()> {
    let removal: Result<()> = (|| -> Result<()> {
        before_remove(directory)?;
        unlink_at(parent, name, true)?;
        Ok(())
    })()
    .with_context(|| operation);
    match removal {
        Ok(()) => Ok(()),
        Err(primary) => match restore_owner_marker(directory, marker_contents) {
            Ok(()) => Err(primary),
            Err(recovery) => Err(PurgeMarkerRecoveryFailure {
                operation,
                primary,
                recovery,
            }
            .into()),
        },
    }
}

fn restore_owner_marker(directory: &File, contents: &str) -> Result<()> {
    let marker_name = c_name(OsStr::new(".owner"))?;
    let mut marker = open_file_at(directory, &marker_name, true)
        .context("cannot restore owner marker without replacing an existing entry")?;
    set_private_file_mode(&marker)?;
    verify_private_file(&marker)?;
    marker
        .write_all(contents.as_bytes())
        .context("cannot write restored owner marker")?;
    marker
        .sync_all()
        .context("cannot sync restored owner marker")?;
    directory
        .sync_all()
        .context("cannot sync directory after owner-marker restoration")?;
    Ok(())
}

#[derive(Debug)]
struct PurgeMarkerRecoveryFailure {
    operation: &'static str,
    primary: anyhow::Error,
    recovery: anyhow::Error,
}

impl std::fmt::Display for PurgeMarkerRecoveryFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} failed ({:#}); owner-marker recovery also failed ({:#})",
            self.operation, self.primary, self.recovery
        )
    }
}

impl std::error::Error for PurgeMarkerRecoveryFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.primary.as_ref())
    }
}

fn is_generation_file(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(value) = name
        .strip_prefix("g-")
        .and_then(|value| value.strip_suffix(".md"))
    else {
        return false;
    };
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_staging_file(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.starts_with(".sillage-staging-") && !name.contains('/'))
}
