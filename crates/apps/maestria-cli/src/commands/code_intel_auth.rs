//! Authorization resolver construction for repository code queries.

use anyhow::Result;
use maestria_blob_fs::FsBlobStore;
use maestria_core::InstanceLayout;
use maestria_domain::CorpusScope;
use maestria_governance::{RetrievalAuthorizationContext, RetrievalSecurityPolicy};
use maestria_ports::{EventFilter, EventLog};
use maestria_retrieval::adapters::{CodeIntelSecurityResolver, CodeIntelSecurityResolverParts};
use maestria_storage_sqlite::SqliteStore;
use std::sync::Arc;

/// Authorization resolver plus its policy context for repository code
/// queries. Query handlers share one construction path so authorization
/// semantics stay identical across symbol, changed, references, and context.
pub(crate) struct CodeIntelAuthorization {
    pub(crate) resolver: CodeIntelSecurityResolver,
    pub(crate) context: RetrievalAuthorizationContext,
}

/// Resolve indexed symbols to durable artifact bindings: authorized symbols
/// pass, unauthorized records are skipped by query handlers (never errors),
/// and unbound sources resolve to `Ok(None)`.
pub(crate) fn code_intel_authorization(layout: &InstanceLayout) -> Result<CodeIntelAuthorization> {
    let store = Arc::new(SqliteStore::open_read_only(&layout.database_path)?);
    let events = EventLog::scan(store.as_ref(), EventFilter { artifact_id: None })?;
    let sources = maestria_domain::active_source_versions(&events);
    let resolver = CodeIntelSecurityResolver::from_events(
        CodeIntelSecurityResolverParts {
            artifacts: store.clone(),
            evidence: store,
            blobs: Arc::new(FsBlobStore::open(&layout.blobs_dir)?),
        },
        &sources,
        &events,
    )
    .map_err(|error| anyhow::anyhow!("prepare repository code authorization: {error}"))?;
    let context = RetrievalSecurityPolicy::default()
        .require_read_allowed(true)
        .allow_unscoped_items(true)
        .authorization_context(&CorpusScope::Global)
        .map_err(|error| anyhow::anyhow!("authorize repository code query: {error}"))?;
    Ok(CodeIntelAuthorization { resolver, context })
}
