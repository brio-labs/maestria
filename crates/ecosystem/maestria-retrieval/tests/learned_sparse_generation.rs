use maestria_domain::{
    ContentHash, CorpusSnapshotId, IndexFingerprint, IndexGeneration, IndexGenerationId,
    IndexGenerationRegistry, IndexLifecycle, RepresentationName, SparseNamespace, TrustZone,
};
use maestria_ports::{SPARSE_REPRESENTATION_V1, SparseFingerprint, SparseIdentity};
use maestria_retrieval::adapters::{
    LearnedSparseGenerationCapability, LearnedSparseGenerationMode,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn fixture_hash(digit: char) -> TestResult<ContentHash> {
    Ok(ContentHash::new(format!(
        "sha256:{}",
        digit.to_string().repeat(64)
    ))?)
}

fn identity() -> TestResult<SparseIdentity> {
    Ok(SparseIdentity {
        generation_id: IndexGenerationId::new(7),
        corpus_snapshot: CorpusSnapshotId::new(11),
        representation: RepresentationName::new(SPARSE_REPRESENTATION_V1),
        namespace: SparseNamespace::new(
            "fixture-instance-a",
            TrustZone::Verified,
            SPARSE_REPRESENTATION_V1,
        )?,
        fingerprint: SparseFingerprint {
            provider: "fixture-local".to_string(),
            model: "fixture-sparse".to_string(),
            revision: "v1".to_string(),
            artifact_hash: fixture_hash('1')?,
            tokenizer_hash: fixture_hash('2')?,
            vocabulary_hash: fixture_hash('3')?,
            vocabulary_size: 65_536,
            term_namespace: "fixture-vocabulary-v1".to_string(),
            query_template_hash: fixture_hash('5')?,
            document_template_hash: fixture_hash('6')?,
            preprocessing_version: "fixture-preprocess-v1".to_string(),
            weighting_version: "fixture-log-frequency-v1".to_string(),
            quantization: "f32".to_string(),
            pruning_threshold: 0.0,
            max_terms: 128,
        },
    })
}

fn registry(identity: &SparseIdentity) -> TestResult<IndexGenerationRegistry> {
    let sparse = &identity.fingerprint;
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: identity.generation_id,
        name: identity.representation.clone(),
        corpus_snapshot: identity.corpus_snapshot,
        sparse_namespace: Some(identity.namespace.clone()),
        fingerprint: IndexFingerprint {
            provider: sparse.provider.clone(),
            model: sparse.model.clone(),
            revision: sparse.revision.clone(),
            artifact_hash: sparse.artifact_hash.clone(),
            dimensions: sparse.vocabulary_size,
            quantization: sparse.quantization.clone(),
            query_template_hash: sparse.query_template_hash.as_str().to_string(),
            document_template_hash: sparse.document_template_hash.as_str().to_string(),
            preprocessing_version: sparse.preprocessing_version.clone(),
        },
        lifecycle: IndexLifecycle::Building,
    })?;
    let _previous_active =
        registry.transition_lifecycle(identity.generation_id, IndexLifecycle::Evaluated)?;
    let _previous_active =
        registry.transition_lifecycle(identity.generation_id, IndexLifecycle::Shadow)?;
    Ok(registry)
}

#[test]
fn shadow_capability_requires_and_accepts_shadow_generation() -> TestResult {
    let identity = identity()?;
    let registry = registry(&identity)?;
    let capability = LearnedSparseGenerationCapability::shadow(&registry, identity.clone())?;
    assert_eq!(capability.mode(), LearnedSparseGenerationMode::Shadow);
    assert!(!capability.is_serving_eligible());
    assert!(LearnedSparseGenerationCapability::activate(&registry, identity).is_err());
    Ok(())
}

#[test]
fn shadow_capability_rejects_cross_instance_and_trust_zone_activation() -> TestResult {
    let identity = identity()?;
    let registry = registry(&identity)?;

    let mut other_instance = identity.clone();
    other_instance.namespace = SparseNamespace::new(
        "fixture-instance-b",
        TrustZone::Verified,
        SPARSE_REPRESENTATION_V1,
    )?;
    assert!(LearnedSparseGenerationCapability::activate(&registry, other_instance).is_err());

    let mut other_trust_zone = identity;
    other_trust_zone.namespace = SparseNamespace::new(
        "fixture-instance-a",
        TrustZone::Untrusted,
        SPARSE_REPRESENTATION_V1,
    )?;
    assert!(LearnedSparseGenerationCapability::activate(&registry, other_trust_zone).is_err());
    Ok(())
}
