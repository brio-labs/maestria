use std::sync::Arc;

use anyhow::{Result, anyhow};
use maestria_core::{InstanceManifest, LateInteractionMode};
use maestria_domain::{IndexGeneration, RealmId, TrustZone};
use maestria_late_local::LocalHttpLateInteractionProvider;
use maestria_ocr_local::LocalHttpOcrProvider;
use maestria_ports::{
    LateInteractionProvider, LearnedSparseProvider, MultiVectorIdentity, OcrIdentity, OcrProvider,
    ProviderDisclosure, RetentionPolicy, SparseIdentity,
};
use maestria_retrieval::{load_late_interaction_identity, validate_late_interaction_generation};
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

/// Builds the local late provider for an explicitly configured shadow or
/// validated-active experiment. Serving policy is applied by runtime assembly.
pub fn build_late_interaction_provider(
    manifest: &InstanceManifest,
    identity: MultiVectorIdentity,
) -> Result<Option<Arc<dyn LateInteractionProvider + Send + Sync>>> {
    let Some(config) = manifest.late_interaction.as_ref() else {
        return Ok(None);
    };
    if matches!(config.mode, LateInteractionMode::Disabled) {
        return Ok(None);
    }
    if !config.profile_path.is_file() {
        return Err(anyhow!(
            "late interaction profile is missing: {}",
            config.profile_path.display()
        ));
    }
    let provider = LocalHttpLateInteractionProvider::new(&config.endpoint, identity)
        .map_err(|error| anyhow!("configure local late provider: {error}"))?;
    if provider.disclosure()
        != Some(ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        })
    {
        return Err(anyhow!(
            "late interaction provider disclosure must be local and no-retention"
        ));
    }
    Ok(Some(Arc::new(provider)))
}

/// Loads and validates the frozen profile against the live generation before
/// constructing the shadow provider.
pub fn build_late_interaction_provider_for_generation(
    manifest: &InstanceManifest,
    generation: &IndexGeneration,
    realm: RealmId,
    trust_zone: TrustZone,
) -> Result<Option<Arc<dyn LateInteractionProvider + Send + Sync>>> {
    let Some(config) = manifest.late_interaction.as_ref() else {
        return Ok(None);
    };
    if matches!(config.mode, LateInteractionMode::Disabled) {
        return Ok(None);
    }
    let identity = load_late_interaction_identity(
        &config.profile_path,
        generation.id,
        generation.corpus_snapshot,
        realm,
        trust_zone,
    )
    .map_err(|error| anyhow!("load late interaction profile: {error}"))?;
    validate_late_interaction_generation(generation, &identity)
        .map_err(|error| anyhow!("validate late interaction generation: {error}"))?;
    build_late_interaction_provider(manifest, identity)
}

/// Reports late-interaction configuration without contacting the sidecar.
pub fn late_interaction_status(manifest: &InstanceManifest) -> Result<String> {
    let Some(config) = manifest.late_interaction.as_ref() else {
        return Ok("disabled (no late-interaction configuration)".to_string());
    };
    if matches!(config.mode, LateInteractionMode::Disabled) {
        return Ok("disabled (late_interaction_mode=disabled)".to_string());
    }
    let profile = if config.profile_path.is_file() {
        "profile=present"
    } else {
        "profile=missing"
    };
    let activation = match config.mode {
        LateInteractionMode::Shadow => "shadow-only",
        LateInteractionMode::Active => "active-requires-promotion-record",
        LateInteractionMode::Disabled => "disabled",
    };
    Ok(format!(
        "configured local endpoint={} {} activation={activation}",
        config.endpoint, profile
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_core::{InstanceManifest, LateInteractionConfig};
    use maestria_domain::{CorpusSnapshotId, IndexGenerationId};
    use std::path::{Path, PathBuf};

    fn profile_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/contracts/late_interaction_profile_v1.json")
    }

    fn manifest(
        mode: LateInteractionMode,
    ) -> Result<InstanceManifest, maestria_domain::RealmIdError> {
        let mut manifest = InstanceManifest::default_for_root(
            PathBuf::from("."),
            RealmId::try_from("a".repeat(64))?,
        );
        manifest.late_interaction = Some(LateInteractionConfig {
            endpoint: "http://127.0.0.1:8093/v1/multivector".to_string(),
            profile_path: profile_path(),
            mode,
        });
        Ok(manifest)
    }

    #[test]
    fn disabled_late_provider_is_not_constructed() -> Result<(), Box<dyn std::error::Error>> {
        let identity = load_late_interaction_identity(
            profile_path(),
            IndexGenerationId::new(1),
            CorpusSnapshotId::new(1),
            RealmId::try_from("a".repeat(64))?,
            TrustZone::Verified,
        )?;
        assert!(
            build_late_interaction_provider(&manifest(LateInteractionMode::Disabled)?, identity)?
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn active_late_provider_constructs_before_request_bound_activation()
    -> Result<(), Box<dyn std::error::Error>> {
        let identity = load_late_interaction_identity(
            profile_path(),
            IndexGenerationId::new(1),
            CorpusSnapshotId::new(1),
            RealmId::try_from("a".repeat(64))?,
            TrustZone::Verified,
        )?;
        assert!(
            build_late_interaction_provider(&manifest(LateInteractionMode::Active)?, identity)?
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn shadow_late_provider_constructs_without_contacting_endpoint()
    -> Result<(), Box<dyn std::error::Error>> {
        let identity = load_late_interaction_identity(
            profile_path(),
            IndexGenerationId::new(1),
            CorpusSnapshotId::new(1),
            RealmId::try_from("a".repeat(64))?,
            TrustZone::Verified,
        )?;
        assert!(
            build_late_interaction_provider(&manifest(LateInteractionMode::Shadow)?, identity)?
                .is_some()
        );
        Ok(())
    }
}
