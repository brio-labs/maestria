use std::sync::Arc;

use anyhow::{Result, anyhow};
use maestria_core::InstanceManifest;
use maestria_ocr_local::LocalHttpOcrProvider;
use maestria_ports::{EmbeddingIdentity, OcrIdentity, OcrProvider, VisualEmbeddingProvider};
use maestria_visual_local::LocalHttpVisualProvider;

pub(crate) fn build_ocr_provider(
    manifest: &InstanceManifest,
) -> Result<Option<Arc<dyn OcrProvider>>> {
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

/// Builds the configured visual provider for an active visual generation.
///
/// The generation identity is supplied by the caller so model vectors cannot
/// be used before the corresponding visual index generation is activated.
pub fn build_visual_provider(
    manifest: &InstanceManifest,
    identity: EmbeddingIdentity,
) -> Result<Option<Arc<dyn VisualEmbeddingProvider + Send + Sync>>> {
    let Some(config) = manifest.visual.as_ref().filter(|config| config.enabled) else {
        return Ok(None);
    };
    if config.remote_provider
        || !matches!(
            config.retention_policy,
            maestria_ports::RetentionPolicy::NoRetention
        )
    {
        return Err(anyhow!(
            "visual provider must be local and no-retention before activation"
        ));
    }
    if identity.fingerprint.model != config.model
        || identity.fingerprint.dimensions != config.dimensions as u32
        || identity.fingerprint.provider != config.provider
        || identity.fingerprint.revision != config.revision
        || identity.fingerprint.preprocessing_version != config.preprocessing_version
        || identity.fingerprint.artifact_hash.as_str() != config.artifact_hash
    {
        return Err(anyhow!(
            "visual provider configuration does not match active generation identity"
        ));
    }
    let provider = LocalHttpVisualProvider::new(&config.endpoint, &config.model, identity)
        .map_err(|error| anyhow!("configure local visual provider: {error}"))?;
    Ok(Some(Arc::new(provider)))
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
