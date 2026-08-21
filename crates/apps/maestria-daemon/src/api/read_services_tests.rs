use super::*;
use crate::test_support::TempDir;
use maestria_core::InstanceManifest;
use maestria_domain::{
    Artifact, ArtifactId, BlobId, Evidence, EvidenceId, EvidenceKind, IndexStatus, LineRange,
    ScopeId, SecurityMetadata, SnapshotRef, ValidationReportId,
};
use maestria_ports::{ArtifactRepository, EvidenceRepository};
use maestria_storage_sqlite::SqliteStore;
use std::collections::BTreeSet;
use std::path::PathBuf;
struct Fixture {
    _temp_dir: TempDir,
    layout: InstanceLayout,
}

fn fixture() -> Result<Fixture> {
    let temp_dir = TempDir::create()?;
    let root = temp_dir.path().to_path_buf();
    let layout = InstanceLayout::for_root(root.clone());
    std::fs::create_dir_all(&layout.system_dir)?;
    if let Some(parent) = layout.database_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manifest = InstanceManifest::default_for_root(root, maestria_test_support::realm_id(10)?);
    std::fs::write(&layout.manifest_path, manifest.encode())?;

    Ok(Fixture {
        _temp_dir: temp_dir,
        layout,
    })
}

#[test]
fn open_evidence_rejects_file_span_outside_current_manifest_roots() -> Result<()> {
    let fixture = fixture()?;
    let store = SqliteStore::open(&fixture.layout.database_path)?;
    let artifact_id = ArtifactId::new(41);
    let evidence_id = EvidenceId::new(42);
    ArtifactRepository::put(
        &store,
        Artifact {
            id: artifact_id,
            title: "outside.md".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Indexed,
            content_hash: Some(maestria_test_support::content_hash(6)?),
            parse_status: None,
            security: maestria_domain::SecurityMetadata::default(),
        },
    )?;
    let outside_path = fixture.layout.root.join("..").join("indexed-outside.md");
    EvidenceRepository::put(
        &store,
        Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: outside_path.display().to_string(),
                range: LineRange::new(1, 1)?,
                snapshot: SnapshotRef::new(BlobId::new(1), maestria_test_support::content_hash(0)?),
            },
            excerpt: "outside".to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: maestria_domain::SecurityMetadata::default(),
        },
    )?;
    drop(store);

    let error = match open_evidence(&fixture.layout, evidence_id.value()) {
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "out-of-scope indexed evidence unexpectedly opened"
            ));
        }
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("outside instance read roots or excluded by policy"),
        "unexpected out-of-scope evidence error: {error:#}"
    );
    Ok(())
}

/// Writes a manifest with both the embedding and the sparse profile enabled
/// so the lane status read reports the configured models and shadow states.
fn write_manifest_with_profiles(layout: &InstanceLayout) -> Result<()> {
    let mut manifest = InstanceManifest::default_for_root(
        layout.root.clone(),
        maestria_test_support::realm_id(10)?,
    );
    manifest.embeddings = Some(maestria_core::EmbeddingConfig {
        enabled: true,
        endpoint: "http://127.0.0.1/v1/embeddings".to_string(),
        model: "test-embedding-model".to_string(),
        dimensions: 384,
        provider: "test-embedding-provider".to_string(),
        revision: "rev-1".to_string(),
        artifact_hash: maestria_test_support::content_hash_str(10),
        preprocessing_version: "v1".to_string(),
        ..maestria_core::EmbeddingConfig::default()
    });
    manifest.sparse = Some(maestria_core::SparseProfileConfig {
        enabled: true,
        endpoint: "http://127.0.0.1/v1/sparse".to_string(),
        provider: "test-sparse-provider".to_string(),
        revision: "rev-1".to_string(),
        artifact_hash: maestria_test_support::content_hash_str(11),
        preprocessing_version: "v1".to_string(),
        model: "test-sparse-model".to_string(),
        vocabulary_size: 1000,
        term_cap: 100,
        remote_provider: false,
        retention_policy: maestria_ports::RetentionPolicy::NoRetention,
    });
    std::fs::write(&layout.manifest_path, manifest.encode())?;
    Ok(())
}

/// Registers and activates a lexical generation so generation resolution
/// succeeds for the read-only status path.
fn seed_lexical_generation(layout: &InstanceLayout) -> Result<()> {
    let mut state = crate::instance_setup::load_kernel_state(layout)?;
    let store = SqliteStore::open(&layout.database_path)?;
    let id = maestria_domain::IndexGenerationId::new(1);
    crate::vector_startup::persist_input(
        &mut state,
        &store,
        maestria_domain::DomainInput::StartIndexGeneration(
            maestria_domain::StartIndexGenerationInput {
                id,
                name: maestria_domain::RepresentationName::new("lexical_text_v1"),
                corpus_snapshot: maestria_domain::DEFAULT_CORPUS_SNAPSHOT_ID,
                fingerprint: maestria_domain::IndexFingerprint {
                    provider: maestria_domain::ProviderName::new("test-provider"),
                    model: maestria_domain::ModelName::new("test-model"),
                    revision: maestria_domain::FingerprintRevision::new("rev-1"),
                    artifact_hash: maestria_test_support::content_hash(1)?,
                    dimensions: 384,
                    quantization: maestria_domain::QuantizationScheme::new("f32"),
                    query_template_hash: maestria_test_support::content_hash(2)?,
                    document_template_hash: maestria_test_support::content_hash(3)?,
                    preprocessing_version: maestria_domain::PreprocessingVersion::new("v1"),
                },
                sparse_namespace: None,
            },
        ),
    )?;
    crate::vector_startup::advance_generation(&mut state, &store, id)?;
    Ok(())
}

fn status_context(layout: InstanceLayout) -> Result<crate::api::server::ApiContext> {
    Ok(crate::api::server::ApiContext {
        layout,
        token: "test-token".to_string(),
        socket_path: PathBuf::new(),
        runtime: None,
        realm_id: maestria_test_support::realm_id(10)?,
    })
}

#[tokio::test]
async fn retrieval_status_reflects_active_hybrid_record() -> Result<()> {
    let fixture = fixture()?;
    write_manifest_with_profiles(&fixture.layout)?;
    seed_lexical_generation(&fixture.layout)?;
    let store = SqliteStore::open(&fixture.layout.database_path)?;
    let hybrid_record = maestria_retrieval::HybridPromotionRecord::new(
        "hybrid-dense-2026-08-09".to_string(),
        "2026-08-09".to_string(),
        BTreeSet::from([maestria_retrieval::LearnedSparseQueryClass::DomainTerminology]),
    )
    .ok_or_else(|| anyhow::anyhow!("hybrid promotion record"))?;
    let record_json = serde_json::to_string(&hybrid_record)?;
    store.save_hybrid_promotion_record(
        "corpus-1",
        "hybrid-dense-2026-08-09",
        "2026-08-09",
        "report-hash-hybrid",
        &record_json,
    )?;
    store.save_promotion_record(
        "corpus-1",
        "sparse-2026-08-09",
        "2026-08-09",
        "report-hash-sparse",
        "{}",
    )?;
    drop(store);

    let context = status_context(fixture.layout)?;
    let response = super::super::search_services::retrieval_status(&context).await?;

    assert_eq!(response.index_generation, 1);
    assert_eq!(response.lanes.hybrid_state, "Active");
    assert_eq!(
        response.lanes.hybrid_served_classes,
        vec!["DomainTerminology".to_string()]
    );
    assert_eq!(
        response.lanes.hybrid_evaluation_id.as_deref(),
        Some("hybrid-dense-2026-08-09")
    );
    assert_eq!(
        response.lanes.hybrid_evaluation_date.as_deref(),
        Some("2026-08-09")
    );
    assert_eq!(
        response.lanes.hybrid_report_hash.as_deref(),
        Some("report-hash-hybrid")
    );
    assert_eq!(
        response.lanes.dense_model.as_deref(),
        Some("test-embedding-model")
    );
    assert_eq!(
        response.lanes.learned_sparse_model.as_deref(),
        Some("test-sparse-model")
    );
    assert!(!response.lanes.dense_enabled);
    let hybrid_wire = response
        .promotion_records
        .hybrid
        .ok_or_else(|| anyhow::anyhow!("hybrid record wire"))?;
    assert_eq!(hybrid_wire.evaluation_id, "hybrid-dense-2026-08-09");
    assert_eq!(hybrid_wire.corpus_id, "corpus-1");
    assert_eq!(hybrid_wire.evaluation_date, "2026-08-09");
    assert_eq!(hybrid_wire.report_hash, "report-hash-hybrid");
    assert!(!hybrid_wire.created_at.is_empty());
    let sparse_wire = response
        .promotion_records
        .learned_sparse
        .ok_or_else(|| anyhow::anyhow!("sparse record wire"))?;
    assert_eq!(sparse_wire.evaluation_id, "sparse-2026-08-09");
    assert_eq!(sparse_wire.report_hash, "report-hash-sparse");
    Ok(())
}

#[tokio::test]
async fn retrieval_status_shadows_without_records() -> Result<()> {
    let fixture = fixture()?;
    write_manifest_with_profiles(&fixture.layout)?;
    seed_lexical_generation(&fixture.layout)?;

    let context = status_context(fixture.layout)?;
    let response = super::super::search_services::retrieval_status(&context).await?;

    assert_eq!(response.lanes.hybrid_state, "Shadow");
    assert!(response.lanes.hybrid_served_classes.is_empty());
    assert!(response.lanes.hybrid_evaluation_id.is_none());
    assert!(response.lanes.hybrid_evaluation_date.is_none());
    assert!(response.lanes.hybrid_report_hash.is_none());
    assert_eq!(response.lanes.learned_sparse_state, "Shadow");
    assert_eq!(response.lanes.repository_code_state, "Shadow");
    assert_eq!(response.lanes.visual_state, "Shadow");
    assert!(response.promotion_records.learned_sparse.is_none());
    assert!(response.promotion_records.hybrid.is_none());
    assert_eq!(
        response.lanes.dense_model.as_deref(),
        Some("test-embedding-model")
    );
    assert_eq!(
        response.lanes.learned_sparse_model.as_deref(),
        Some("test-sparse-model")
    );
    Ok(())
}

#[test]
fn open_evidence_rejects_indexed_non_file_evidence_from_other_scope() -> Result<()> {
    let fixture = fixture()?;
    let store = SqliteStore::open(&fixture.layout.database_path)?;
    let artifact_id = ArtifactId::new(51);
    let evidence_id = EvidenceId::new(52);
    ArtifactRepository::put(
        &store,
        Artifact {
            id: artifact_id,
            title: "validation-report".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Indexed,
            content_hash: None,
            parse_status: None,
            security: SecurityMetadata {
                scope_id: Some(ScopeId::new(1)),
                ..SecurityMetadata::default()
            },
        },
    )?;
    EvidenceRepository::put(
        &store,
        Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::Validation {
                report_id: ValidationReportId::new(7),
            },
            excerpt: "validation report".to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: SecurityMetadata {
                scope_id: Some(ScopeId::new(99)),
                ..SecurityMetadata::default()
            },
        },
    )?;
    drop(store);

    let error = match open_evidence(&fixture.layout, evidence_id.value()) {
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "cross-instance indexed non-file evidence unexpectedly opened"
            ));
        }
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("evidence is not available under retrieval policy: Scope mismatch"),
        "unexpected cross-instance evidence error: {error:#}"
    );
    Ok(())
}
