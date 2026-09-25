//! Shared read-only evidence store assembly.
//!
//! The daemon's evidence API and the CLI's `evidence` command both assemble
//! the same five-store stack (SQLite + blob store + full-text index + parser
//! registry + core services with no vector/graph index). The two copies
//! drifted (R28), and the CLI copy opened SQLite read-write for read-only
//! work (R32). Both entry points delegate here. The scoped open functions are
//! the single enforcement point for the instance's read scope (R48): every
//! client surface evaluates the retrieval policy and the manifest read-root
//! scope before evidence is dispatched.

use std::{fs, path::PathBuf};

use anyhow::{Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{
    InstanceLayout, InstanceManifest, OpenChunkEvidenceInput, OpenEvidenceInput, lexical_normalize,
    path_matches_pattern,
};
use maestria_domain::{
    ActiveSourceVersions, ArtifactId, ArtifactVersionId, ChunkId, ContentHash, Evidence,
    EvidenceId, EvidenceKind,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_parsers::ParserRegistry;
use maestria_ports::{EventFilter, EventLog, EvidenceRepository};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;

use crate::blocked_patterns::runtime_blocked_patterns;

type ScopedEvidenceOpen = Result<(maestria_core::OpenEvidenceOutput, Option<PathBuf>)>;

fn complete_scoped_batch(outputs: Vec<Option<ScopedEvidenceOpen>>) -> Vec<ScopedEvidenceOpen> {
    outputs
        .into_iter()
        .map(|output| match output {
            Some(output) => output,
            None => Err(anyhow!("evidence preview was not opened")),
        })
        .collect()
}

/// The store stack backing read-only evidence retrieval.
pub struct EvidenceStores {
    pub sqlite: SqliteStore,
    pub blobs: FsBlobStore,
    pub search_index: TantivyFullTextIndex,
    pub parser: ParserRegistry,
}

/// Open the read-only SQLite store for evidence lookups.
///
/// Handlers that must reject out-of-scope evidence before opening heavy
/// adapters (blob store, full-text index) open this first, look up, and only
/// then call [`complete_evidence_stores`].
pub fn open_evidence_sqlite(layout: &InstanceLayout) -> Result<SqliteStore> {
    Ok(SqliteStore::open_read_only(&layout.database_path)?)
}

/// Complete the evidence store stack around an already-open SQLite store.
///
/// Opens the blob store and the full-text index without its writer lock:
/// evidence retrieval never mutates either.
pub fn complete_evidence_stores(
    layout: &InstanceLayout,
    sqlite: SqliteStore,
) -> Result<EvidenceStores> {
    let blobs = FsBlobStore::open(&layout.blobs_dir)?;
    let search_index = TantivyFullTextIndex::open_read_only(&layout.full_text_index_dir)?;
    let parser = ParserRegistry::with_defaults();
    Ok(EvidenceStores {
        sqlite,
        blobs,
        search_index,
        parser,
    })
}

/// Wire the evidence store stack into core services with no vector or graph
/// index, borrowing from `stores`.
pub fn evidence_core_services(stores: &EvidenceStores) -> maestria_core::CoreServices<'_> {
    maestria_core::CoreServices::new(maestria_core::CorePorts {
        artifacts: &stores.sqlite,
        chunks: &stores.sqlite,
        cards: &stores.sqlite,
        evidence: &stores.sqlite,
        events: &stores.sqlite,
        parser: &stores.parser,
        search_index: &stores.search_index,
        blobs: &stores.blobs,
        vector_index: None,
        graph_index: None,
    })
}

/// The read-side retrieval policy every client surface applies before
/// dispatching an evidence open (R48): read-allowed security metadata is
/// required, scoped items (web-sourced evidence) must belong to the instance
/// scope, and locally ingested items are unscoped by construction and remain
/// readable under the instance's local-first baseline.
fn evidence_retrieval_policy() -> RetrievalSecurityPolicy {
    RetrievalSecurityPolicy::default()
        .require_read_allowed(true)
        .required_scope(maestria_domain::DEFAULT_INSTANCE_SCOPE_ID)
        .allow_unscoped_items(true)
}

fn evidence_retrieval_authorization() -> Result<maestria_governance::RetrievalAuthorizationContext>
{
    evidence_retrieval_policy()
        .authorization_context(&maestria_domain::CorpusScope::Global)
        .map_err(|error| anyhow!("evidence retrieval policy is not authorized: {error}"))
}

/// Open evidence by id after enforcing the instance's read scope and retrieval
/// policy (R48). Shared by the daemon API handler and the CLI command so the
/// two client surfaces cannot drift.
pub fn open_evidence_scoped(
    layout: &InstanceLayout,
    evidence_id: u64,
) -> Result<maestria_core::OpenEvidenceOutput> {
    open_evidence_scoped_with_authorization(
        layout,
        evidence_id,
        evidence_retrieval_authorization()?,
    )
}

/// Open evidence with a caller-composed authorization context that has already
/// passed provider-side federation governance.
pub fn open_evidence_scoped_with_authorization(
    layout: &InstanceLayout,
    evidence_id: u64,
    authorization: maestria_governance::RetrievalAuthorizationContext,
) -> Result<maestria_core::OpenEvidenceOutput> {
    open_evidence_scoped_with_authorization_and_pdf_path(layout, evidence_id, authorization, None)
        .map(|(output, _)| output)
}

/// Open evidence after current-source validation and return its scoped PDF
/// source path when the evidence refers to a PDF.
pub fn open_evidence_scoped_with_authorization_and_pdf_path(
    layout: &InstanceLayout,
    evidence_id: u64,
    authorization: maestria_governance::RetrievalAuthorizationContext,
    allowed_roots: Option<&[PathBuf]>,
) -> Result<(maestria_core::OpenEvidenceOutput, Option<PathBuf>)> {
    let mut results = open_evidence_scoped_batch_with_authorization_and_pdf_path(
        layout,
        &[evidence_id],
        &authorization,
        None,
        allowed_roots,
    )?;
    match results.pop() {
        Some(result) => result,
        None => Err(anyhow!("evidence open returned no result")),
    }
}

/// Open multiple evidence records with one scoped, read-only store assembly.
///
/// Each requested evidence item keeps an independent result: a stale or
/// unauthorized candidate can be omitted without preventing safe previews for
/// other candidates in the same search response.
pub fn open_evidence_scoped_batch_with_authorization(
    layout: &InstanceLayout,
    evidence_ids: &[u64],
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    current_sources: Option<(i64, &ActiveSourceVersions)>,
    allowed_roots: Option<&[PathBuf]>,
) -> Result<Vec<Result<maestria_core::OpenEvidenceOutput>>> {
    Ok(open_evidence_scoped_batch_with_authorization_and_pdf_path(
        layout,
        evidence_ids,
        authorization,
        current_sources,
        allowed_roots,
    )?
    .into_iter()
    .map(|result| result.map(|(output, _)| output))
    .collect())
}

fn open_evidence_scoped_batch_with_authorization_and_pdf_path(
    layout: &InstanceLayout,
    evidence_ids: &[u64],
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    current_sources: Option<(i64, &ActiveSourceVersions)>,
    allowed_roots: Option<&[PathBuf]>,
) -> Result<Vec<ScopedEvidenceOpen>> {
    if evidence_ids.is_empty() {
        return Ok(Vec::new());
    }

    let manifest = decode_manifest(layout)?;
    let sqlite = open_evidence_sqlite(layout)?;
    let mut outputs: Vec<Option<ScopedEvidenceOpen>> = std::iter::repeat_with(|| None)
        .take(evidence_ids.len())
        .collect();
    let mut pending = Vec::with_capacity(evidence_ids.len());

    for (index, evidence_id) in evidence_ids.iter().copied().enumerate() {
        let evidence = match EvidenceRepository::get(&sqlite, EvidenceId::new(evidence_id)) {
            Ok(Some(evidence)) => evidence,
            Ok(None) => {
                outputs[index] = Some(Err(anyhow!("evidence not found")));
                continue;
            }
            Err(error) => {
                outputs[index] = Some(Err(anyhow!("read evidence for preview: {error}")));
                continue;
            }
        };
        let validation = reject_denied(authorization.evaluate(&evidence.security), "evidence")
            .and_then(|()| validate_evidence_scope(&manifest, &evidence));
        if let Err(error) = validation {
            outputs[index] = Some(Err(error));
        } else {
            pending.push((index, evidence_id, evidence));
        }
    }

    if pending.is_empty() {
        return Ok(complete_scoped_batch(outputs));
    }
    let active_sources = if let Some((revision, sources)) = current_sources {
        if sqlite.searchable_source_revision()? != revision {
            return Err(anyhow!(
                "indexed source versions changed before evidence preview"
            ));
        }
        std::borrow::Cow::Borrowed(sources)
    } else {
        let events = EventLog::scan(&sqlite, EventFilter { artifact_id: None })
            .map_err(|error| anyhow!("read active sources for evidence: {error}"))?;
        std::borrow::Cow::Owned(maestria_domain::active_source_versions(&events))
    };
    let mut current_pending = Vec::with_capacity(pending.len());
    for (index, evidence_id, evidence) in pending {
        if let Err(error) = current_source_path_with_active_sources(
            layout,
            &manifest,
            &active_sources,
            &evidence,
            allowed_roots,
        ) {
            outputs[index] = Some(Err(error));
        } else {
            current_pending.push((index, evidence_id));
        }
    }
    if current_pending.is_empty() {
        return Ok(complete_scoped_batch(outputs));
    }

    let stores = complete_evidence_stores(layout, sqlite)?;
    let core = evidence_core_services(&stores);
    for (index, evidence_id) in current_pending {
        let opened = core
            .open_evidence_pre_authorized(
                OpenEvidenceInput {
                    evidence_id: EvidenceId::new(evidence_id),
                },
                authorization,
            )
            .map_err(|error| anyhow!(error))
            .and_then(|output| {
                validate_evidence_scope(&manifest, &output.evidence)?;
                let source_path = current_source_path_with_active_sources(
                    layout,
                    &manifest,
                    &active_sources,
                    &output.evidence,
                    allowed_roots,
                )?;
                let pdf_source_path = match &output.evidence.kind {
                    EvidenceKind::PdfSpan { .. } | EvidenceKind::PdfRegion { .. } => source_path,
                    _ => None,
                };
                Ok((output, pdf_source_path))
            });
        outputs[index] = Some(opened);
    }

    Ok(complete_scoped_batch(outputs))
}

/// Open chunk evidence after enforcing the instance's read scope and retrieval
/// policy (R48). Shared by the daemon API handler and the CLI command.
pub fn open_chunk_evidence_scoped(
    layout: &InstanceLayout,
    chunk_id: u64,
) -> Result<maestria_core::OpenEvidenceOutput> {
    let manifest = decode_manifest(layout)?;
    let authorization = evidence_retrieval_authorization()?;
    let sqlite = open_evidence_sqlite(layout)?;
    let stores = complete_evidence_stores(layout, sqlite)?;
    let core = evidence_core_services(&stores);
    let output = core.open_chunk_evidence_pre_authorized(
        OpenChunkEvidenceInput {
            chunk_id: ChunkId::new(chunk_id),
        },
        &authorization,
    )?;
    validate_evidence_scope(&manifest, &output.evidence)?;
    validate_current_source(layout, &manifest, &stores.sqlite, &output.evidence)?;
    Ok(output)
}

fn decode_manifest(layout: &InstanceLayout) -> Result<InstanceManifest> {
    InstanceManifest::decode(&fs::read_to_string(&layout.manifest_path)?)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))
}

fn reject_denied(decision: maestria_governance::RetrievalDecision, subject: &str) -> Result<()> {
    match decision {
        maestria_governance::RetrievalDecision::Denied(reason) => Err(anyhow!(
            "{subject} is not available under retrieval policy: {reason}"
        )),
        maestria_governance::RetrievalDecision::Allowed => Ok(()),
    }
}

/// Reject evidence whose source path is outside the manifest read roots or
/// matches a blocked pattern (R48).
pub fn validate_evidence_scope(manifest: &InstanceManifest, evidence: &Evidence) -> Result<()> {
    let path = match &evidence.kind {
        EvidenceKind::FileSpan { path, .. } | EvidenceKind::DocxParagraphSpan { path, .. } => path,
        _ => return Ok(()),
    };
    if source_scope_allowed(manifest, path) {
        return Ok(());
    }
    Err(anyhow!(
        "evidence source path {path} is outside instance read roots or excluded by policy"
    ))
}

fn validate_current_source(
    layout: &InstanceLayout,
    manifest: &InstanceManifest,
    sqlite: &SqliteStore,
    evidence: &Evidence,
) -> Result<()> {
    let events = EventLog::scan(sqlite, EventFilter { artifact_id: None })
        .map_err(|error| anyhow!("read active sources for evidence: {error}"))?;
    let active_sources = maestria_domain::active_source_versions(&events);
    current_source_path_with_active_sources(layout, manifest, &active_sources, evidence, None)
        .map(|_| ())
}

fn current_source_path_with_active_sources(
    layout: &InstanceLayout,
    manifest: &InstanceManifest,
    active_sources: &ActiveSourceVersions,
    evidence: &Evidence,
    allowed_roots: Option<&[PathBuf]>,
) -> Result<Option<PathBuf>> {
    let expected_local_path =
        match &evidence.kind {
            EvidenceKind::FileSpan { path, .. } | EvidenceKind::DocxParagraphSpan { path, .. } => {
                let path = std::path::Path::new(path);
                let path = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    manifest.root.join(path)
                };
                Some(lexical_normalize(&path).ok_or_else(|| {
                    anyhow!("evidence source changed or was removed after indexing")
                })?)
            }
            _ => None,
        };
    let expected_source_hash = match &evidence.kind {
        EvidenceKind::FileSpan { snapshot, .. }
        | EvidenceKind::DocxParagraphSpan { snapshot, .. }
        | EvidenceKind::PdfSpan { snapshot, .. }
        | EvidenceKind::PdfRegion { snapshot, .. } => Some(snapshot.content_hash()),
        _ => None,
    };
    let matches_source =
        |path: &PathBuf,
         (artifact_id, _, content_hash): &(ArtifactId, ArtifactVersionId, ContentHash)| {
            artifact_id == &evidence.artifact_id
                && expected_local_path
                    .as_ref()
                    .is_none_or(|expected| lexical_normalize(path).as_ref() == Some(expected))
                && expected_source_hash
                    .is_none_or(|expected| content_hash.as_str() == expected.as_str())
        };
    // File and DOCX evidence names its source path. Look it up directly in
    // large active-source snapshots; retain the full scan for non-normalized
    // event paths and PDF evidence, which has no path in its citation.
    let source = expected_local_path
        .as_ref()
        .and_then(|expected| active_sources.get_key_value(expected))
        .filter(|(path, version)| matches_source(path, version))
        .or_else(|| {
            active_sources
                .iter()
                .find(|(path, version)| matches_source(path, version))
        });
    let Some((path, (_, _, content_hash))) = source else {
        if matches!(
            &evidence.kind,
            EvidenceKind::FileSpan { .. }
                | EvidenceKind::DocxParagraphSpan { .. }
                | EvidenceKind::PdfSpan { .. }
                | EvidenceKind::PdfRegion { .. }
        ) {
            return Err(anyhow!(
                "evidence source changed or was removed after indexing"
            ));
        }
        if allowed_roots.is_some() {
            return Err(anyhow!("source not selected for consumer read roots"));
        }
        return Ok(None);
    };
    if !crate::search_executor::path_is_within_allowed_roots(allowed_roots, path) {
        return Err(anyhow!("source not selected for consumer read roots"));
    }
    if !manifest.allows_source(path) || maestria_index_selection::is_privacy_excluded_path(path) {
        return Err(anyhow!(
            "evidence source changed or was removed after indexing"
        ));
    }
    let source_path = path.display().to_string();
    let fresh = crate::watcher::fresh_source_paths(
        layout,
        manifest,
        &[(source_path.clone(), content_hash.as_str().to_owned())],
    );
    if fresh.contains(&source_path) {
        Ok(Some(path.clone()))
    } else {
        Err(anyhow!(
            "evidence source changed or was removed after indexing"
        ))
    }
}

fn source_scope_allowed(manifest: &InstanceManifest, path: &str) -> bool {
    let path = std::path::Path::new(path);
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(normalized) = lexical_normalize(path) {
        candidates.push(normalized);
    }
    if path.is_relative()
        && let Some(normalized) = lexical_normalize(&manifest.root.join(path))
    {
        candidates.push(normalized);
    }
    let roots: Vec<_> = manifest
        .read_roots
        .iter()
        .filter_map(|root| lexical_normalize(root))
        .collect();
    let blocked_patterns = runtime_blocked_patterns(manifest);
    candidates.iter().any(|candidate| {
        roots.iter().any(|root| candidate.starts_with(root))
            && !blocked_patterns
                .iter()
                .any(|pattern| path_matches_pattern(candidate, pattern))
            && !maestria_index_selection::is_privacy_excluded_path(candidate)
    })
}
