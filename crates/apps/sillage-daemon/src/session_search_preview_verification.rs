use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::Result;
use sillage_domain::{
    ActiveSourceVersions, ArtifactVersionId, ContentHash, Evidence, EvidenceKind, LineRange,
    SnapshotRef, SourceChoice,
};
use tokio_util::sync::CancellationToken;

use super::{
    deadline::SessionDeadline,
    ingest,
    retained::{RetainedBinding, RetainedSources},
};
use crate::SearchRuntime;

pub(super) struct PreviewContext<'a> {
    pub(super) runtime: &'a SearchRuntime,
    pub(super) stores: &'a ingest::SessionStores,
    pub(super) choice: &'a SourceChoice,
    pub(super) selected_path: &'a Path,
    pub(super) retained_sources: &'a RetainedSources,
    pub(super) manifest: &'a sillage_core::InstanceManifest,
    pub(super) authorization: &'a sillage_governance::RetrievalAuthorizationContext,
    pub(super) cancellation: &'a CancellationToken,
    pub(super) deadline: SessionDeadline,
    pub(super) verification_work: Mutex<u64>,
}

pub(super) struct SelectedPreviewSource<'a> {
    pub(super) evidence: &'a Evidence,
    pub(super) evidence_range: &'a LineRange,
    pub(super) binding: &'a RetainedBinding,
    pub(super) path: PathBuf,
    pub(super) version: ArtifactVersionId,
    pub(super) hash: ContentHash,
}

pub(super) fn select_current_preview_source<'a>(
    context: &'a PreviewContext<'_>,
    sources: &ActiveSourceVersions,
    evidence: &'a Evidence,
    expected_version: ArtifactVersionId,
) -> Result<SelectedPreviewSource<'a>> {
    let (evidence_path, evidence_range, evidence_snapshot) = file_span(evidence).ok_or(
        super::StaleEvidence("session evidence is not a Markdown file span"),
    )?;
    let (path, version, hash) = sources
        .iter()
        .find(|(path, (artifact_id, version, _))| {
            *artifact_id == evidence.artifact_id
                && *version == expected_version
                && path.as_path() == Path::new(evidence_path)
        })
        .map(|(path, (_, version, hash))| (path.clone(), *version, hash.clone()))
        .ok_or(super::StaleEvidence(
            "session evidence source version is no longer current",
        ))?;
    if !ingest::scope_contains(context.choice.scope(), context.selected_path, &path)
        || !context.manifest.allows_source(&path)
        || sillage_index_selection::is_privacy_excluded_path(&path)
    {
        return Err(
            super::StaleEvidence("session evidence source is outside its selected scope").into(),
        );
    }
    if evidence_snapshot.content_hash() != &hash {
        return Err(super::StaleEvidence(
            "session evidence snapshot does not match the active source version",
        )
        .into());
    }
    let binding = context
        .retained_sources
        .binding(evidence.artifact_id, version, &path)?;
    Ok(SelectedPreviewSource {
        evidence,
        evidence_range,
        binding,
        path,
        version,
        hash,
    })
}

pub(super) fn verify_latest_preview_state(
    context: &PreviewContext<'_>,
    selected: &SelectedPreviewSource<'_>,
    output: &sillage_core::OpenEvidenceOutput,
    expected_source_revision: i64,
) -> Result<i64> {
    let (latest_revision, latest_sources) = context.runtime.interactive_current_sources()?;
    if latest_sources
        .get(&selected.path)
        .is_none_or(|(artifact, latest_version, latest_hash)| {
            *artifact != output.artifact.id
                || *latest_version != selected.version
                || latest_hash != &selected.hash
        })
    {
        return Err(super::StaleEvidence(
            "session source identity changed while opening its preview",
        )
        .into());
    }
    let source_revision = context.runtime.event_log.searchable_source_revision()?;
    if source_revision != expected_source_revision || latest_revision != expected_source_revision {
        return Err(
            super::StaleEvidence("session source revision changed while opening evidence").into(),
        );
    }
    Ok(source_revision)
}

pub(super) fn file_span(evidence: &Evidence) -> Option<(&str, &LineRange, &SnapshotRef)> {
    match &evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            snapshot,
        } => Some((path, range, snapshot)),
        _ => None,
    }
}
