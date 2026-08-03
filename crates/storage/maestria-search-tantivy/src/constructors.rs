use crate::{
    error::to_port_error,
    schema::{self, schema},
    tantivy_index::TantivyFullTextIndex,
};
use maestria_domain::{ContentHash, IndexFingerprint, content_hash};
use maestria_ports::PortError;
use std::path::Path;
use tantivy::Index;

impl TantivyFullTextIndex {
    /// Return the deterministic fingerprint of the lexical index definition.
    pub fn fingerprint(&self) -> Result<IndexFingerprint, PortError> {
        let schema_hash = content_hash(schema::CANONICAL_SCHEMA.as_bytes());
        let revision = env!("CARGO_PKG_VERSION").to_string();
        let artifact_hash =
            ContentHash::new(schema_hash.clone()).map_err(|error| PortError::InternalContext {
                context: "invalid Tantivy schema fingerprint",
                source: error.to_string(),
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

    pub fn in_memory() -> Result<Self, PortError> {
        Self::from_index(Index::create_in_ram(schema()), false, None, false)
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
