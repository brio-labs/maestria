use anyhow::Result;
use maestria_daemon::ingestion_policy::{is_privacy_excluded_path, is_supported_source_file};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn collect_index_files(path: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
    if is_excluded_index_path(path) {
        return Err(anyhow::anyhow!(
            "index path is excluded by privacy policy: {}",
            path.display()
        ));
    }
    if is_symlink(path)? {
        return Err(anyhow::anyhow!(
            "index path is a symlink and is not indexed: {}",
            path.display()
        ));
    }
    if path.is_file() {
        if !is_supported_index_path(path) {
            return Err(anyhow::anyhow!(
                "unsupported index file type: {}",
                path.display()
            ));
        }
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        return Err(anyhow::anyhow!(
            "index path does not exist: {}",
            path.display()
        ));
    }
    if !recursive {
        return Err(anyhow::anyhow!(
            "{} is a directory; pass --recursive to index contained files",
            path.display()
        ));
    }

    let mut files = Vec::new();
    let walker = ignore::WalkBuilder::new(path)
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .require_git(false)
        .follow_links(false)
        .build();

    for result in walker {
        let entry = result?;
        let entry_path = entry.path();
        if let Some(error) = entry.error() {
            return Err(anyhow::anyhow!(
                "index traversal failed at {}: {error}",
                entry_path.display()
            ));
        }

        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_symlink())
        {
            continue;
        }
        if is_excluded_index_path(entry_path) {
            continue;
        }

        if entry_path.is_file() && is_supported_index_path(entry_path) {
            files.push(entry_path.to_path_buf());
        }
    }

    files.sort();
    Ok(files)
}

pub(crate) fn is_excluded_index_path(path: &Path) -> bool {
    is_privacy_excluded_path(path)
}

fn is_symlink(path: &Path) -> Result<bool> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

pub(crate) fn is_supported_index_path(path: &Path) -> bool {
    is_supported_source_file(path)
}
