use std::sync::Arc;

use anyhow::{Result, anyhow};
use maestria_core::InstanceManifest;
use maestria_ocr_local::LocalHttpOcrProvider;
use maestria_ports::{LearnedSparseProvider, OcrIdentity, OcrProvider, SparseIdentity};
use maestria_sparse_local::LocalHttpSparseProvider;

pub(crate) fn build_ocr_provider(
    manifest: &InstanceManifest,
) -> Result<Option<Arc<dyn OcrProvider + Send + Sync>>> {
    let Some(config) = manifest.ocr.as_ref().filter(|config| config.enabled) else {
        return Ok(None);
    };
    let identity = OcrIdentity {
        provider: config.provider.clone(),
        model: config.model.clone(),
        revision: config.revision.clone(),
        artifact_hash: config.artifact_hash.clone(),
        preprocessing_version: config.preprocessing_version.clone(),
    };
    let provider = LocalHttpOcrProvider::new(&config.endpoint, &config.model, identity)
        .map_err(|error| anyhow!("configure local OCR provider: {error}"))?;
    Ok(Some(Arc::new(provider)))
}

/// Builds the configured learned-sparse provider for an active sparse generation.
///
/// The generation identity is supplied by the caller so sparse vectors cannot
/// be produced before the corresponding sparse index generation is activated.
pub fn build_sparse_provider(
    manifest: &InstanceManifest,
    identity: SparseIdentity,
) -> Result<Option<Arc<dyn LearnedSparseProvider + Send + Sync>>> {
    let Some(config) = manifest.sparse.as_ref().filter(|config| config.enabled) else {
        return Ok(None);
    };
    if identity.fingerprint.model.as_str() != config.model
        || identity.fingerprint.provider.as_str() != config.provider
        || identity.fingerprint.revision.as_str() != config.revision
        || identity.fingerprint.preprocessing_version.as_str() != config.preprocessing_version
        || identity.fingerprint.artifact_hash.as_str() != config.artifact_hash
        || identity.fingerprint.vocabulary_size != config.vocabulary_size
        || identity.fingerprint.max_terms != config.term_cap
    {
        return Err(anyhow!(
            "sparse provider configuration does not match active generation identity"
        ));
    }
    let provider = LocalHttpSparseProvider::new(&config.endpoint, &config.model, identity)
        .map_err(|error| anyhow!("configure local sparse provider: {error}"))?;
    let expected = maestria_ports::ProviderDisclosure {
        remote: config.remote_provider,
        retention: config.retention_policy.clone(),
    };
    if provider.disclosure() != Some(expected) {
        return Err(anyhow!(
            "sparse provider disclosure does not match manifest expectation"
        ));
    }
    Ok(Some(Arc::new(provider)))
}

/// Reports sparse capability without touching the model endpoint.
pub fn sparse_status(manifest: &InstanceManifest) -> Result<String> {
    let Some(config) = manifest.sparse.as_ref() else {
        return Ok("disabled (no sparse configuration)".to_string());
    };
    if !config.enabled {
        return Ok("disabled (sparse_enabled=false)".to_string());
    }
    if config.remote_provider
        || !matches!(
            config.retention_policy,
            maestria_ports::RetentionPolicy::NoRetention
        )
    {
        return Ok(format!(
            "configured but rejected (provider={} model={} requires local no-retention)",
            config.provider, config.model
        ));
    }
    Ok(format!(
        "configured local provider={} model={} endpoint={} activation=requires-fingerprinted-sparse-generation",
        config.provider, config.model, config.endpoint
    ))
}

/// Reports visual capability without touching the model endpoint.
pub fn visual_status(manifest: &InstanceManifest) -> Result<String> {
    let Some(config) = manifest.visual.as_ref() else {
        return Ok("disabled (no visual configuration)".to_string());
    };
    if !config.enabled {
        return Ok("disabled (visual_enabled=false)".to_string());
    }
    if config.remote_provider
        || !matches!(
            config.retention_policy,
            maestria_ports::RetentionPolicy::NoRetention
        )
    {
        return Ok(format!(
            "configured but rejected (provider={} model={} requires local no-retention)",
            config.provider, config.model
        ));
    }
    Ok(format!(
        "configured local provider={} model={} endpoint={} activation=requires-fingerprinted-visual-generation",
        config.provider, config.model, config.endpoint
    ))
}

pub fn ocr_status(manifest: &InstanceManifest) -> Result<String> {
    let Some(config) = manifest.ocr.as_ref() else {
        return Ok("disabled (no ocr configuration)".to_string());
    };
    if !config.enabled {
        return Ok("disabled (ocr_enabled=false)".to_string());
    }
    let identity = OcrIdentity {
        provider: config.provider.clone(),
        model: config.model.clone(),
        revision: config.revision.clone(),
        artifact_hash: config.artifact_hash.clone(),
        preprocessing_version: config.preprocessing_version.clone(),
    };
    let provider = LocalHttpOcrProvider::new(&config.endpoint, &config.model, identity)
        .map_err(|error| anyhow!("configure local OCR provider: {error}"))?;
    match provider.check_local_tools() {
        Ok(()) => Ok(format!(
            "configured local provider={} model={} endpoint={} rasterizer=ready",
            config.provider, config.model, config.endpoint
        )),
        Err(error) => Ok(format!(
            "configured local provider={} model={} endpoint={} rasterizer=unavailable: {}",
            config.provider, config.model, config.endpoint, error
        )),
    }
}
