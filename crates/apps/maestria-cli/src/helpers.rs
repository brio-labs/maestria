use anyhow::{Context, Result};
use maestria_core::InstanceLayout;
use std::{
    fs,
    path::{Path, PathBuf},
};

use maestria_core::InstanceManifest;
use maestria_governance::PrivacyExclusions;

pub(crate) fn ensure_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    maestria_daemon::prepare_instance(instance_dir)
}

pub(crate) fn validated_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    let layout = InstanceLayout::for_root(instance_dir);
    if !layout.manifest_path.exists() {
        return Err(anyhow::anyhow!(
            "instance manifest is missing at {}; run init first",
            layout.manifest_path.display()
        ));
    }
    load_manifest(&layout)?;
    Ok(layout)
}

pub(crate) fn load_manifest(layout: &InstanceLayout) -> Result<InstanceManifest> {
    let contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    maestria_core::InstanceService::parse_manifest(&contents)
        .map_err(|error| anyhow::anyhow!("parse instance manifest: {error}"))
}

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
    collect_files_recursive(path, &mut files)?;
    files.sort();
    Ok(files)
}

pub(crate) fn collect_files_recursive(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink()
            || is_excluded_index_path(&path)
            || (path.is_file() && !is_supported_index_path(&path))
        {
            continue;
        }
        if path.is_dir() {
            collect_files_recursive(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

pub(crate) fn is_excluded_index_path(path: &Path) -> bool {
    let default_exclusions = PrivacyExclusions::default();
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        matches!(
            name.as_ref(),
            ".ssh" | ".gnupg" | "node_modules" | "target" | "dist" | "build"
        ) || name.starts_with(".env.")
    }) || default_exclusions.is_excluded(path)
}

fn is_symlink(path: &Path) -> Result<bool> {
    Ok(fs::symlink_metadata(path)?.file_type().is_symlink())
}

pub(crate) fn is_supported_index_path(path: &Path) -> bool {
    if path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml") {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown" | "txt" | "text" | "rs" | "pdf")
    )
}

pub(crate) fn source_label(evidence: &maestria_domain::Evidence) -> String {
    use maestria_domain::EvidenceKind;

    match &evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            content_hash,
            ..
        } => format!(
            "source=file path={} lines={}-{} hash={}",
            path, range.start, range.end, content_hash
        ),
        EvidenceKind::PdfSpan {
            blob,
            page_start,
            page_end,
        } => format!("source=pdf blob={} pages={}-{}", blob, page_start, page_end),
        EvidenceKind::WebSnapshot { url, snapshot, .. } => {
            format!("source=web url={} snapshot={}", url, snapshot)
        }
        EvidenceKind::CommandOutput {
            harness_run,
            stream,
            blob,
        } => format!(
            "source=command run={} stream={:?} blob={}",
            harness_run, stream, blob
        ),
        EvidenceKind::TestResult {
            harness_run,
            status,
            log,
        } => format!(
            "source=test run={} status={:?} log={}",
            harness_run, status, log
        ),
        EvidenceKind::Diff {
            harness_run,
            patch_blob,
        } => format!("source=diff run={} patch={}", harness_run, patch_blob),
        EvidenceKind::Validation { report_id } => {
            format!("source=validation report={}", report_id)
        }
    }
}

pub(crate) fn is_db_locked(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}").to_lowercase();
    message.contains("database is locked")
        || message.contains("database is busy")
        || message.contains("locked")
}
