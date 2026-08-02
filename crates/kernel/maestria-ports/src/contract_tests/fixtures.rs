pub fn search_budget(
    limit: u64,
) -> Result<maestria_domain::SearchExecutionBudget, maestria_domain::SearchCompatibilityError> {
    maestria_domain::SearchExecutionBudget::new(limit, 10_000, 100_000, 0)
}

/// Build a fixture embedding identity for contract and adapter tests.
///
/// The identity carries a fabricated artifact hash and generation id because
/// tests exercise transport, encoding, and retrieval behavior, not provider
/// identity provenance; generation-aware providers construct real identities
/// through their own configuration paths.
pub fn fixture_embedding_identity(
    model: &str,
    dimensions: usize,
) -> Result<crate::EmbeddingIdentity, crate::PortError> {
    let artifact_hash = maestria_domain::ContentHash::new(format!("sha256:{}", "0".repeat(64)))
        .map_err(|error| crate::PortError::InternalContext {
            context: "create fixture embedding fingerprint",
            source: error.to_string(),
        })?;
    let template_hash = |digit: u8| {
        maestria_domain::ContentHash::new(format!("sha256:{}", format!("{digit:x}").repeat(64)))
            .map_err(|error| crate::PortError::InternalContext {
                context: "create fixture template hash",
                source: error.to_string(),
            })
    };
    Ok(crate::EmbeddingIdentity {
        generation_id: maestria_domain::IndexGenerationId::new(1),
        fingerprint: maestria_domain::IndexFingerprint {
            provider: "fixture-local".to_string(),
            model: model.to_string(),
            revision: "fixture".to_string(),
            artifact_hash,
            dimensions: dimensions as u32,
            quantization: "f32".to_string(),
            query_template_hash: template_hash(3)?,
            document_template_hash: template_hash(4)?,
            preprocessing_version: "fixture".to_string(),
        },
        representation: maestria_domain::RepresentationName::new("dense_text_v1"),
    })
}
