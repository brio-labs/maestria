use crate::{
    error::to_port_error,
    migration,
    ram_directory::BudgetedRamDirectory,
    ram_memory::{InMemoryIndexError, RamMemoryBudget},
    schema,
    tantivy_index::{TantivyFullTextIndex, WriterMemoryUsage},
};
use sillage_domain::{ContentHash, IndexFingerprint, content_hash};
use sillage_ports::PortError;
use std::{path::Path, sync::Arc};
use tantivy::{Index, IndexSettings};

impl TantivyFullTextIndex {
    /// Return the deterministic fingerprint of the lexical index definition.
    pub fn fingerprint(&self) -> Result<IndexFingerprint, PortError> {
        let schema_hash = content_hash(schema::CANONICAL_SCHEMA.as_bytes());
        let revision = env!("CARGO_PKG_VERSION").to_string();
        let artifact_hash = ContentHash::new(schema_hash.clone()).map_err(|error| {
            PortError::internal("invalid Tantivy schema fingerprint", error.to_string())
        })?;
        Ok(IndexFingerprint {
            provider: sillage_domain::ProviderName::new("tantivy"),
            model: sillage_domain::ModelName::new("lexical"),
            revision: sillage_domain::FingerprintRevision::new(revision),
            artifact_hash,
            dimensions: 0,
            quantization: sillage_domain::QuantizationScheme::new("f32"),
            query_template_hash: ContentHash::new(content_hash(b"query: {{text}}")).map_err(
                |error| PortError::InternalContext {
                    context: "invalid query template hash",
                    source: error.to_string(),
                },
            )?,
            document_template_hash: ContentHash::new(content_hash(b"doc: {{text}}")).map_err(
                |error| PortError::InternalContext {
                    context: "invalid document template hash",
                    source: error.to_string(),
                },
            )?,
            preprocessing_version: sillage_domain::PreprocessingVersion::new(
                "tantivy-default-tokenizer-v1",
            ),
        })
    }

    /// Resolve the lexical fingerprint without taking a writer lock for a current index.
    ///
    /// A missing index is initialized and a recognized legacy schema is migrated. Errors from
    /// opening or inspecting any other existing index are returned instead of being retried
    /// through a writable open.
    pub fn fingerprint_for_path(path: impl AsRef<Path>) -> Result<IndexFingerprint, PortError> {
        let path = path.as_ref();
        match std::fs::symlink_metadata(path.join("meta.json")) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let index = Self::open(path)?;
                let fingerprint = index.fingerprint()?;
                drop(index);
                return Ok(fingerprint);
            }
            Err(error) => {
                return Err(PortError::downstream(
                    "read full-text index metadata",
                    format!("{}: {error}", path.display()),
                ));
            }
        }

        let index = Index::open_in_dir(path).map_err(to_port_error)?;
        let index_schema = index.schema();
        if migration::schema_has_cards(&index_schema)
            && schema::supports_filtered_queries(&index_schema)
            && migration::schema_has_lexical(&index_schema)
        {
            let marker = path.join(".cards-rebuild");
            let index = Self::from_index(index, marker.exists(), Some(marker), true)?;
            return index.fingerprint();
        }
        if !migration::schema_supports_legacy_migration(&index_schema) {
            return Err(PortError::internal(
                "fingerprint full-text index",
                "index schema is neither current nor migratable from stored chunks",
            ));
        }

        drop(index);
        let index = Self::open(path)?;
        let fingerprint = index.fingerprint()?;
        drop(index);
        Ok(fingerprint)
    }

    /// Create a canonical RAM index under a shared logical admission quota.
    /// Writer, native-state, and retained/output charges are estimates, not RSS enforcement or
    /// hard encoded-size bounds. Tantivy's writer estimate must be at least 15,000,000 bytes.
    /// If output exceeds an estimate while the global quota is exhausted, native writing can
    /// fail after admission, terminally for that accepted work. An ordinary preflight refusal
    /// retains credits for previously accepted work. There is no automatic rollback, replay,
    /// or fallback.
    pub fn in_memory_with_budget(
        budget: Arc<RamMemoryBudget>,
        writer_budget_bytes: usize,
    ) -> Result<Self, InMemoryIndexError> {
        let writer_bytes = u64::try_from(writer_budget_bytes)
            .map_err(|_| crate::MemoryBudgetError::AccountingOverflow)?;
        let directory = BudgetedRamDirectory::new(Arc::clone(&budget))?;
        let writer_reservation = budget.reserve(writer_bytes)?;
        let maximum_pre_document_table_bytes = (1_u64 << 19)
            .checked_mul(16)
            .ok_or(crate::MemoryBudgetError::AccountingOverflow)?
            .min(writer_bytes.saturating_sub(1_000_000));
        let rehash_bytes = maximum_pre_document_table_bytes
            .checked_mul(2)
            .ok_or(crate::MemoryBudgetError::AccountingOverflow)?;
        let writer_memory = WriterMemoryUsage::new(
            writer_bytes,
            rehash_bytes,
            budget.reserve(
                rehash_bytes
                    .checked_add(65_536)
                    .ok_or(crate::MemoryBudgetError::AccountingOverflow)?,
            )?,
        );
        let settings = IndexSettings {
            docstore_compress_dedicated_thread: false,
            ..IndexSettings::default()
        };
        let index = Index::create(directory.clone(), schema::schema(), settings);
        if let Some(error) = directory.take_memory_error() {
            return Err(error.into());
        }
        let index = index.map_err(to_port_error)?;
        Self::from_budgeted_ram_index(
            index,
            directory,
            budget,
            writer_budget_bytes,
            writer_reservation,
            writer_memory,
        )
    }
    /// Open an existing lexical index without acquiring Tantivy's writer lock.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, PortError> {
        let path = path.as_ref();
        if !path.join("meta.json").exists() {
            return Err(PortError::DownstreamContext {
                context: "read-only full-text index directory missing meta.json",
                source: path.display().to_string(),
            });
        }
        let index = Index::open_in_dir(path).map_err(to_port_error)?;
        let marker = path.join(".cards-rebuild");
        Self::from_index(index, marker.exists(), Some(marker), true)
    }
}
