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

use std::fs;

use anyhow::{Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{InstanceLayout, InstanceManifest, OpenChunkEvidenceInput, OpenEvidenceInput};
use maestria_domain::{ChunkId, Evidence, EvidenceId, EvidenceKind};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_parsers::ParserRegistry;
use maestria_ports::{ArtifactRepository, ChunkRepository, EvidenceRepository};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;

use crate::blocked_patterns::runtime_blocked_patterns;

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

/// Open the evidence store stack for an instance layout.
///
/// Eager variant for entry points that need the full stack up front. The
/// SQLite store is opened read-only and the full-text index without its
/// writer lock: evidence retrieval never mutates either.
pub fn open_evidence_stores(layout: &InstanceLayout) -> Result<EvidenceStores> {
    let sqlite = open_evidence_sqlite(layout)?;
    complete_evidence_stores(layout, sqlite)
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
    let manifest = decode_manifest(layout)?;
    let sqlite = open_evidence_sqlite(layout)?;
    let evidence_id = EvidenceId::new(evidence_id);
    if let Some(evidence) = EvidenceRepository::get(&sqlite, evidence_id)? {
        reject_denied(authorization.evaluate(&evidence.security), "evidence")?;
        validate_evidence_scope(&manifest, &evidence)?;
        if let Some(artifact) = ArtifactRepository::get(&sqlite, evidence.artifact_id)? {
            reject_denied(authorization.evaluate(&artifact.security), "artifact")?;
        }
    }
    let stores = complete_evidence_stores(layout, sqlite)?;
    let core = evidence_core_services(&stores);
    let output =
        core.open_evidence_pre_authorized(OpenEvidenceInput { evidence_id }, &authorization)?;
    Ok(output)
}

/// Open chunk evidence after enforcing the instance's read scope and retrieval
/// policy (R48). Shared by the daemon API handler and the CLI command.
pub fn open_chunk_evidence_scoped(
    layout: &InstanceLayout,
    chunk_id: u64,
) -> Result<maestria_core::OpenEvidenceOutput> {
    let manifest = decode_manifest(layout)?;
    let sqlite = open_evidence_sqlite(layout)?;
    let chunk_id = ChunkId::new(chunk_id);
    let chunk = ChunkRepository::get(&sqlite, chunk_id)?
        .ok_or_else(|| anyhow!("chunk {chunk_id} does not exist"))?;
    let evidence = EvidenceRepository::get(
        &sqlite,
        maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order),
    )?
    .ok_or_else(|| anyhow!("evidence for chunk {chunk_id} does not exist"))?;
    if evidence.artifact_id != chunk.artifact_id {
        return Err(anyhow!(
            "chunk evidence belongs to artifact {}, requested chunk belongs to artifact {}",
            evidence.artifact_id,
            chunk.artifact_id
        ));
    }
    let authorization = evidence_retrieval_authorization()?;
    reject_denied(authorization.evaluate(&evidence.security), "evidence")?;
    validate_evidence_scope(&manifest, &evidence)?;
    if let Some(artifact) = ArtifactRepository::get(&sqlite, evidence.artifact_id)? {
        reject_denied(authorization.evaluate(&artifact.security), "artifact")?;
    }
    let stores = complete_evidence_stores(layout, sqlite)?;
    let core = evidence_core_services(&stores);
    let output = core
        .open_chunk_evidence_pre_authorized(OpenChunkEvidenceInput { chunk_id }, &authorization)?;
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
    let EvidenceKind::FileSpan { path, .. } = &evidence.kind else {
        return Ok(());
    };
    if source_scope_allowed(manifest, path) {
        return Ok(());
    }
    Err(anyhow!(
        "evidence source path {path} is outside instance read roots or excluded by policy"
    ))
}

fn source_scope_allowed(manifest: &InstanceManifest, path: &str) -> bool {
    let path = std::path::Path::new(path);
    let mut candidates = vec![lexical_normalize(path)];
    if path.is_relative() {
        candidates.push(lexical_normalize(&manifest.root.join(path)));
    }
    let roots: Vec<_> = manifest
        .read_roots
        .iter()
        .map(|root| lexical_normalize(root))
        .collect();
    let blocked_patterns = runtime_blocked_patterns(manifest);
    candidates.iter().any(|candidate| {
        roots.iter().any(|root| candidate.starts_with(root))
            && !blocked_patterns
                .iter()
                .any(|pattern| path_matches_pattern(candidate, pattern))
    })
}

fn lexical_normalize(path: &std::path::Path) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn path_matches_pattern(path: &std::path::Path, pattern: &str) -> bool {
    path.components()
        .any(|component| glob_matches(&component.as_os_str().to_string_lossy(), pattern))
}

fn glob_matches(value: &str, pattern: &str) -> bool {
    let value: Vec<char> = value.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let mut value_index = 0usize;
    let mut pattern_index = 0usize;
    let mut star_pattern_index = None;
    let mut star_value_index = 0usize;

    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == '?' || pattern[pattern_index] == value[value_index])
        {
            value_index += 1;
            pattern_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            star_pattern_index = Some(pattern_index);
            star_value_index = value_index;
            pattern_index += 1;
        } else if let Some(star_index) = star_pattern_index {
            pattern_index = star_index + 1;
            star_value_index += 1;
            value_index = star_value_index;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}
