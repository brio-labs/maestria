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
    let artifact_hash = sillage_test_support::content_hash(0).map_err(|error| {
        crate::PortError::InternalContext {
            context: "create fixture embedding fingerprint",
            source: error.to_string(),
        }
    })?;
    let template_hash = |digit: u8| {
        sillage_domain::ContentHash::new(format!("sha256:{}", format!("{digit:x}").repeat(64)))
            .map_err(|error| crate::PortError::InternalContext {
                context: "create fixture template hash",
                source: error.to_string(),
            })
    };
    Ok(crate::EmbeddingIdentity {
        generation_id: sillage_domain::IndexGenerationId::new(1),
        fingerprint: sillage_domain::IndexFingerprint {
            provider: sillage_domain::ProviderName::new("fixture-local"),
            model: sillage_domain::ModelName::new(model),
            revision: sillage_domain::FingerprintRevision::new("fixture"),
            artifact_hash,
            dimensions: dimensions as u32,
            quantization: sillage_domain::QuantizationScheme::new("f32"),
            query_template_hash: template_hash(3)?,
            document_template_hash: template_hash(4)?,
            preprocessing_version: sillage_domain::PreprocessingVersion::new("fixture"),
        },
        representation: sillage_domain::RepresentationName::new("dense_text_v1"),
    })
}
