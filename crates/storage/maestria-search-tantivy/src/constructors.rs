use crate::{error::to_port_error, migration, schema, tantivy_index::TantivyFullTextIndex};
use maestria_domain::{ContentHash, IndexFingerprint, content_hash};
use maestria_ports::PortError;
use std::path::Path;
use tantivy::Index;

impl TantivyFullTextIndex {
    /// Return the deterministic fingerprint of the lexical index definition.
    pub fn fingerprint(&self) -> Result<IndexFingerprint, PortError> {
        let schema_hash = content_hash(schema::CANONICAL_SCHEMA.as_bytes());
        let revision = env!("CARGO_PKG_VERSION").to_string();
        let artifact_hash = ContentHash::new(schema_hash.clone()).map_err(|error| {
            PortError::internal("invalid Tantivy schema fingerprint", error.to_string())
        })?;
        Ok(IndexFingerprint {
            provider: maestria_domain::ProviderName::new("tantivy"),
            model: maestria_domain::ModelName::new("lexical"),
            revision: maestria_domain::FingerprintRevision::new(revision),
            artifact_hash,
            dimensions: 0,
            quantization: maestria_domain::QuantizationScheme::new("f32"),
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
            preprocessing_version: maestria_domain::PreprocessingVersion::new(
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

    #[cfg(test)]
    pub fn in_memory() -> Result<Self, PortError> {
        Self::from_index(Index::create_in_ram(schema::schema()), false, None, false)
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
