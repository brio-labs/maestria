//! Instance manifest unit tests (shared behavior family).

use super::*;

use std::path::Path;
fn test_realm_id() -> Result<RealmId, Box<dyn std::error::Error>> {
    Ok(RealmId::try_from("a".repeat(64))?)
}

#[test]
fn manifest_round_trips_ordered_roots_and_exclusions() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = InstanceManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        realm_id: test_realm_id()?,
        root: PathBuf::from("/tmp/instance"),
        read_roots: vec![PathBuf::from("/tmp/notes"), PathBuf::from("/tmp/project")],
        excluded_patterns: vec![".env".to_string(), "*.key".to_string()],
        embeddings: None,
        ocr: None,
        visual: None,
        sparse: None,
    };

    let decoded = InstanceManifest::decode(&manifest.encode())?;
    assert_eq!(decoded, manifest);
    Ok(())
}

#[test]
fn embedding_configuration_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = InstanceManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        realm_id: test_realm_id()?,
        root: PathBuf::from("/tmp/instance"),
        read_roots: vec![PathBuf::from("/tmp/instance")],
        excluded_patterns: vec![".env".to_string()],
        embeddings: Some(EmbeddingConfig {
            enabled: true,
            endpoint: "http://127.0.0.1:8080/v1/embeddings".to_string(),
            model: "local-model".to_string(),
            dimensions: 3,
            provider: "local".to_string(),
            revision: "v1".to_string(),
            artifact_hash:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            preprocessing_version: "v1".to_string(),
            remote_provider: false,
            retention_policy: RetentionPolicy::NoRetention,
        }),
        ocr: None,
        visual: None,
        sparse: None,
    };

    assert_eq!(InstanceManifest::decode(&manifest.encode())?, manifest);
    Ok(())
}

#[test]
fn ocr_configuration_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = InstanceManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        realm_id: test_realm_id()?,
        root: PathBuf::from("/tmp/instance"),
        read_roots: vec![PathBuf::from("/tmp/instance")],
        excluded_patterns: vec![".env".to_string()],
        embeddings: None,
        ocr: Some(OcrConfig {
            enabled: true,
            endpoint: "http://127.0.0.1:10000/v1/chat/completions".to_string(),
            model: "Unlimited-OCR".to_string(),
            provider: "baidu".to_string(),
            revision: "main".to_string(),
            artifact_hash:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            preprocessing_version: "pdf-pdftoppm-v1".to_string(),
        }),
        visual: None,
        sparse: None,
    };
    assert_eq!(InstanceManifest::decode(&manifest.encode())?, manifest);
    Ok(())
}

#[test]
fn visual_configuration_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = InstanceManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        realm_id: test_realm_id()?,
        root: PathBuf::from("/tmp/instance"),
        read_roots: vec![PathBuf::from("/tmp/instance")],
        excluded_patterns: vec![".env".to_string()],
        embeddings: None,
        ocr: None,
        visual: Some(VisualConfig {
            enabled: true,
            endpoint: "http://127.0.0.1:10001/v1/embeddings".to_string(),
            model: "siglip-base-patch16-224".to_string(),
            dimensions: 768,
            provider: "siglip-onnx".to_string(),
            revision: "4649052661e53c7000355844105f8a1792088239".to_string(),
            artifact_hash:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            preprocessing_version: "siglip-224-rgb-v1".to_string(),
            remote_provider: false,
            retention_policy: RetentionPolicy::NoRetention,
        }),
        sparse: None,
    };

    assert_eq!(InstanceManifest::decode(&manifest.encode())?, manifest);
    Ok(())
}
#[test]
fn sparse_configuration_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = InstanceManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        realm_id: test_realm_id()?,
        root: PathBuf::from("/tmp/instance"),
        read_roots: vec![PathBuf::from("/tmp/instance")],
        excluded_patterns: vec![".env".to_string()],
        embeddings: None,
        ocr: None,
        visual: None,
        sparse: Some(SparseProfileConfig {
            enabled: true,
            endpoint: "http://127.0.0.1:10002/v1/sparse".to_string(),
            provider: "splade-onnx".to_string(),
            revision: "762be6a7206e2f299182705972a65e5c46e62be2".to_string(),
            artifact_hash:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            preprocessing_version: "splade-templates-v1".to_string(),
            model: "prithivida/Splade_PP_en_v1".to_string(),
            vocabulary_size: 30_522,
            term_cap: 256,
            remote_provider: false,
            retention_policy: RetentionPolicy::NoRetention,
        }),
    };
    assert_eq!(InstanceManifest::decode(&manifest.encode())?, manifest);
    Ok(())
}

#[test]
fn sparse_configuration_rejects_remote_provider() -> Result<(), Box<dyn std::error::Error>> {
    let contents = "schema_version=2\nrealm_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nsparse_enabled=true\n\
            sparse_endpoint=http://127.0.0.1:10002/v1/sparse\nsparse_model=splade\n\
            sparse_provider=splade-onnx\nsparse_revision=v1\n\
            sparse_artifact_hash=sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
            sparse_preprocessing_version=v1\nsparse_vocabulary_size=30522\nsparse_term_cap=256\n\
            sparse_remote_provider=true\n";
    let result = InstanceManifest::decode(contents);
    assert!(matches!(result, Err(CoreError::InvalidManifest { .. })));
    Ok(())
}

#[test]
fn sparse_configuration_rejects_retained_retention() -> Result<(), Box<dyn std::error::Error>> {
    let contents = "schema_version=2\nrealm_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nsparse_enabled=true\n\
            sparse_endpoint=http://127.0.0.1:10002/v1/sparse\nsparse_model=splade\n\
            sparse_provider=splade-onnx\nsparse_revision=v1\n\
            sparse_artifact_hash=sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
            sparse_preprocessing_version=v1\nsparse_vocabulary_size=30522\nsparse_term_cap=256\n\
            sparse_retention_policy=provider_defined\n";
    let result = InstanceManifest::decode(contents);
    assert!(matches!(result, Err(CoreError::InvalidManifest { .. })));
    Ok(())
}

#[test]
fn sparse_configuration_rejects_remote_endpoint() -> Result<(), Box<dyn std::error::Error>> {
    let contents = "schema_version=2\nrealm_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nsparse_enabled=true\n\
            sparse_endpoint=https://example.com/v1/sparse\nsparse_model=splade\n\
            sparse_provider=splade-onnx\nsparse_revision=v1\n\
            sparse_artifact_hash=sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
            sparse_preprocessing_version=v1\nsparse_vocabulary_size=30522\nsparse_term_cap=256\n";
    let result = InstanceManifest::decode(contents);
    assert!(matches!(result, Err(CoreError::InvalidManifest { .. })));
    Ok(())
}

#[test]
fn sparse_configuration_rejects_term_cap_beyond_vocabulary()
-> Result<(), Box<dyn std::error::Error>> {
    let contents = "schema_version=2\nrealm_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nsparse_enabled=true\n\
            sparse_endpoint=http://127.0.0.1:10002/v1/sparse\nsparse_model=splade\n\
            sparse_provider=splade-onnx\nsparse_revision=v1\n\
            sparse_artifact_hash=sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
            sparse_preprocessing_version=v1\nsparse_vocabulary_size=256\nsparse_term_cap=512\n";
    let result = InstanceManifest::decode(contents);
    assert!(matches!(result, Err(CoreError::InvalidManifest { .. })));
    Ok(())
}

#[test]
fn sparse_configuration_rejects_missing_fingerprint() -> Result<(), Box<dyn std::error::Error>> {
    let contents = "schema_version=2\nrealm_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nsparse_enabled=true\n\
            sparse_endpoint=http://127.0.0.1:10002/v1/sparse\nsparse_model=splade\n";
    let result = InstanceManifest::decode(contents);
    assert!(matches!(result, Err(CoreError::InvalidManifest { .. })));
    Ok(())
}

#[test]
fn migration_requires_explicit_realm_identity_and_preserves_v1_scope()
-> Result<(), Box<dyn std::error::Error>> {
    let v1 = "schema_version=1\nroot=/tmp/instance\nread_root=/tmp/notes\n\
            read_root=/tmp/project\nexcluded_pattern=.env\nexcluded_pattern=*.key\n";
    assert!(InstanceManifest::decode(v1).is_err());

    let migrated = InstanceManifest::migrate_v1(v1, test_realm_id()?)?;
    assert_eq!(migrated.schema_version, MANIFEST_SCHEMA_VERSION);
    assert_eq!(migrated.realm_id, test_realm_id()?);
    assert_eq!(
        migrated.read_roots,
        vec![PathBuf::from("/tmp/notes"), PathBuf::from("/tmp/project")]
    );
    assert_eq!(InstanceManifest::decode(&migrated.encode())?, migrated);
    Ok(())
}

#[test]
fn embedding_configuration_rejects_remote_endpoint() -> Result<(), Box<dyn std::error::Error>> {
    let contents = "schema_version=2\nrealm_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nembedding_enabled=true\n\
            embedding_endpoint=https://example.com/v1/embeddings\n\
            embedding_model=remote\nembedding_dimensions=3\n";
    let result = InstanceManifest::decode(contents);
    assert!(matches!(result, Err(CoreError::InvalidManifest { .. })));
    Ok(())
}

#[test]
fn embedding_configuration_rejects_partial_values() -> Result<(), Box<dyn std::error::Error>> {
    let contents = "schema_version=2\nrealm_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nembedding_enabled=true\n";
    let result = InstanceManifest::decode(contents);
    assert!(matches!(result, Err(CoreError::InvalidManifest { .. })));
    Ok(())
}

#[test]
fn default_manifest_scopes_reads_to_instance_root() -> Result<(), Box<dyn std::error::Error>> {
    let manifest =
        InstanceManifest::default_for_root(PathBuf::from("/tmp/instance"), test_realm_id()?);
    assert_eq!(manifest.read_roots, vec![PathBuf::from("/tmp/instance")]);
    assert!(manifest.excluded_patterns.iter().any(|item| item == ".env"));
    Ok(())
}

#[test]
fn source_scope_rejects_escape_and_sensitive_paths() -> Result<(), Box<dyn std::error::Error>> {
    let manifest =
        InstanceManifest::default_for_root(PathBuf::from("/tmp/instance"), test_realm_id()?);
    assert!(manifest.allows_source(Path::new("/tmp/instance/notes.md")));
    assert!(!manifest.allows_source(Path::new("/tmp/instance/../outside.md")));
    assert!(!manifest.allows_source(Path::new("/tmp/instance/.env.local")));
    assert!(!manifest.allows_source(Path::new("/tmp/other/notes.md")));
    Ok(())
}

#[test]
fn source_scope_rejects_relative_escape_above_root() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = InstanceManifest::default_for_root(PathBuf::from("workspace"), test_realm_id()?);
    assert!(manifest.allows_source(Path::new("workspace/notes.md")));
    // `..` above the root must not collapse into an in-scope path.
    assert!(!manifest.allows_source(Path::new("../workspace/notes.md")));
    assert!(!manifest.allows_source(Path::new("workspace/../outside.md")));
    assert!(!manifest.allows_source(Path::new("../secret.md")));
    Ok(())
}
