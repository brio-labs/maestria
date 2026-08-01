//! Shared read-only evidence store assembly.
//!
//! The daemon's evidence API and the CLI's `evidence` command both assemble
//! the same five-store stack (SQLite + blob store + full-text index + parser
//! registry + core services with no vector/graph index). The two copies
//! drifted (R28), and the CLI copy opened SQLite read-write for read-only
//! work (R32). Both entry points delegate here.

use anyhow::Result;
use maestria_blob_fs::FsBlobStore;
use maestria_core::InstanceLayout;
use maestria_parsers::ParserRegistry;
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;

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
