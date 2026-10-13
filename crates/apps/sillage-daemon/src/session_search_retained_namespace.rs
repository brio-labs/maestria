use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path},
};

use anyhow::{Context, Result};

use super::{
    c_name, mkdir_at, open_directory_at, open_file_at, open_filesystem_root,
    set_private_directory_mode, set_private_file_mode, verify_private_directory,
    verify_private_file,
};

pub(in crate::session_search::retained) fn open_store_root(
    root: &Path,
    create: bool,
) -> Result<Option<File>> {
    if !root.is_absolute()
        || root.as_os_str().is_empty()
        || root
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        anyhow::bail!("retention store root must be absolute and normalized");
    }
    let mut components = root
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .peekable();
    if components.peek().is_none() {
        anyhow::bail!("filesystem root cannot be used as a retention store");
    }
    let mut current = open_filesystem_root()?;
    while let Some(component) = components.next() {
        let name = c_name(component)?;
        let is_final = components.peek().is_none();
        let mut created = false;
        let child = match open_directory_at(&current, &name) {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => {
                return Ok(None);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match mkdir_at(&current, &name, super::super::STORE_DIRECTORY_MODE) {
                    Ok(()) => created = true,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
                open_directory_at(&current, &name)?
            }
            Err(error) => return Err(error.into()),
        };
        if created {
            set_private_directory_mode(&child)?;
        }
        if is_final {
            verify_private_directory(&child)?;
        }
        current = child;
    }
    Ok(Some(current))
}

pub(in crate::session_search::retained) fn open_owned_child(
    parent: &File,
    name: &str,
    marker: &str,
    contents: &str,
) -> Result<Option<File>> {
    let name = c_name(std::ffi::OsStr::new(name))?;
    let directory = match open_directory_at(parent, &name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    verify_private_directory(&directory)?;
    verify_marker(&directory, marker, contents)?;
    Ok(Some(directory))
}

pub(in crate::session_search::retained) fn ensure_owned_directory(
    parent: &File,
    name: &str,
    marker: &str,
    contents: &str,
) -> Result<File> {
    let name = c_name(std::ffi::OsStr::new(name))?;
    let created = match mkdir_at(parent, &name, super::super::STORE_DIRECTORY_MODE) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => return Err(error.into()),
    };
    let directory = open_directory_at(parent, &name)?;
    if created {
        set_private_directory_mode(&directory)?;
        create_marker(&directory, marker, contents)?;
    } else {
        verify_private_directory(&directory)?;
        verify_marker(&directory, marker, contents)?;
    }
    Ok(directory)
}

fn create_marker(directory: &File, marker: &str, contents: &str) -> Result<()> {
    let marker = c_name(std::ffi::OsStr::new(marker))?;
    let mut file = open_file_at(directory, &marker, true)
        .context("cannot create private retention ownership marker")?;
    set_private_file_mode(&file)?;
    verify_private_file(&file)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    directory.sync_all()?;
    Ok(())
}

pub(in crate::session_search::retained) fn verify_marker(
    directory: &File,
    marker: &str,
    contents: &str,
) -> Result<()> {
    if contents.len() > super::super::STORE_MARKER_MAX_BYTES {
        anyhow::bail!("retention ownership marker definition exceeds its fixed bound");
    }
    let marker = c_name(std::ffi::OsStr::new(marker))?;
    let mut file = open_file_at(directory, &marker, false)?;
    verify_private_file(&file)?;
    let expected = contents.as_bytes();
    let metadata = file.metadata()?;
    if metadata.len() != u64::try_from(expected.len())? {
        anyhow::bail!("retention ownership marker has an unexpected length");
    }
    let mut actual = Vec::new();
    actual.try_reserve_exact(expected.len())?;
    actual.resize(expected.len(), 0);
    file.read_exact(&mut actual)?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 || actual.as_slice() != expected {
        anyhow::bail!("retention ownership marker does not match this source store");
    }
    Ok(())
}
