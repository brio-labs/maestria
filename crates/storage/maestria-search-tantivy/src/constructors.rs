use super::{TantivyFullTextIndex, schema};
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
            ContentHash::new(schema_hash.clone()).map_err(|error| PortError::Internal {
                message: format!("invalid Tantivy schema fingerprint: {error}"),
            })?;
        Ok(IndexFingerprint {
            provider: "tantivy".to_string(),
            model: "lexical".to_string(),
            revision,
            artifact_hash,
            dimensions: 0,
            quantization: "f32".to_string(),
            query_template_hash: content_hash(b"query: {{text}}"),
            document_template_hash: content_hash(b"doc: {{text}}"),
            preprocessing_version: "tantivy-default-tokenizer-v1".to_string(),
        })
    }

    pub fn in_memory() -> Result<Self, PortError> {
        Self::from_index(Index::create_in_ram(schema()), false, None, false)
    }

    /// Open an existing lexical index without acquiring Tantivy's writer lock.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, PortError> {
        let path = path.as_ref();
        if !path.join("meta.json").exists() {
            return Err(PortError::Downstream {
                message: format!("full-text index is not initialized: {}", path.display()),
            });
        }
        let index = Index::open_in_dir(path).map_err(super::to_port_error)?;
        let marker = path.join(".cards-rebuild");
        Self::from_index(index, marker.exists(), Some(marker), true)
    }
}
