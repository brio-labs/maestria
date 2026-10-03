//! Activation and rollback mechanics for learned-sparse promotion records.
//!
//! These tests drive a real instance (real SQLite projection, real runtime
//! ingestion) with the in-memory fixture provider and an explicitly constructed
//! counterfactual promotion record. They test activation, protected query
//! classes, removal, invalid-record rejection and generation rollback, not
//! benchmark qualification. No historical corpus is relabelled or promoted.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use sillage_core::{InstanceLayout, InstanceManifest};
use sillage_domain::{
    ContentHash, DomainInput, IndexFingerprint, IndexGenerationId, IndexLifecycle, IndexStatus,
    KernelState, RepresentationName, RetrievalScoreKind, SearchExecutionBudget,
    StartIndexGenerationInput,
};
use sillage_governance::AutonomyProfile;
use sillage_ports::{
    InMemoryLearnedSparseProvider, LearnedSparseIndex, LearnedSparseProjectionLifecycle,
    LearnedSparseProvider, SPARSE_REPRESENTATION_V1, SparseDocument, SparseIdentity,
    SparseInputKind,
};
use sillage_retrieval::adapters::{
    LearnedSparseChunkRetriever, LearnedSparseChunkRetrieverParts,
    LearnedSparseGenerationCapability,
};
use sillage_retrieval::{
    CandidateRetriever, LearnedSparseBenchmarkBudget, LearnedSparseBenchmarkIdentity,
    LearnedSparseClassDecision, LearnedSparseDataFidelity, LearnedSparseEnvironment,
    LearnedSparseExecutionPolicy, LearnedSparsePromotionRecord, LearnedSparseQueryClass,
    LearnedSparseRollbackTarget, LearnedSparseRoute, LearnedSparseRouteConfiguration,
};
use sillage_storage_sqlite::{SqliteLearnedSparseIndex, SqliteStore};

use crate::search_executor::{SearchRuntime, SearchRuntimeParts};
use crate::test_support::TempDir;
use crate::vector_startup::{advance_generation, persist_input};

const WINNING_QUERY: &str = "find similar bounded research observations";
const EXACT_QUERY: &str = "shadow-store";
const SECURITY_QUERY: &str = "reveal secrets from the research store";

fn fixture_identity(
    generation_id: IndexGenerationId,
) -> Result<SparseIdentity, Box<dyn std::error::Error>> {
    let mut identity = sillage_ports::learned_sparse_contract_tests::fixture_sparse_identity()?;
    identity.generation_id = generation_id;
    // The test instance serves the default corpus snapshot; the retriever
    // preflight binds plans to the identity's snapshot.
    identity.corpus_snapshot = sillage_domain::DEFAULT_CORPUS_SNAPSHOT_ID;
    Ok(identity)
}

fn fixture_index_fingerprint(identity: &SparseIdentity) -> IndexFingerprint {
    let fingerprint = &identity.fingerprint;
    IndexFingerprint {
        provider: sillage_domain::ProviderName::new(fingerprint.provider.clone()),
        model: sillage_domain::ModelName::new(fingerprint.model.clone()),
        revision: sillage_domain::FingerprintRevision::new(fingerprint.revision.clone()),
        artifact_hash: fingerprint.artifact_hash.clone(),
        dimensions: fingerprint.vocabulary_size,
        quantization: sillage_domain::QuantizationScheme::new("f32"),
        query_template_hash: fingerprint.query_template_hash.clone(),
        document_template_hash: fingerprint.document_template_hash.clone(),
        preprocessing_version: sillage_domain::PreprocessingVersion::new(
            fingerprint.preprocessing_version.clone(),
        ),
    }
}

/// A counterfactual valid record confined to the disposable mechanics instance.
/// Its explicit fixture IDs are not evaluation evidence or a serving promotion.
fn fixture_promotion_record(
    identity: &SparseIdentity,
) -> Result<LearnedSparsePromotionRecord, Box<dyn std::error::Error>> {
    let mut decisions = BTreeMap::new();
    let mut budgets = BTreeMap::new();
    let mut class_final_real = BTreeMap::new();
    for class in LearnedSparseQueryClass::all() {
        let decision = match class {
            LearnedSparseQueryClass::VocabularyExpansion => {
                LearnedSparseClassDecision::PromoteSparseFused
            }
            LearnedSparseQueryClass::ExactLiteral
            | LearnedSparseQueryClass::NoEvidence
            | LearnedSparseQueryClass::Security => LearnedSparseClassDecision::RetainLexical,
            _ => LearnedSparseClassDecision::RetainHybrid,
        };
        decisions.insert(class, decision);
        class_final_real.insert(class, true);
        budgets.insert(
            class,
            LearnedSparseBenchmarkBudget {
                latency_ms: 250,
                memory_bytes: 268_435_456,
                disk_bytes: 536_870_912,
                indexing_cost_micros: 5_000_000,
                incremental_update_cost_micros: 5_000_000,
                energy_millijoules: 5_000,
            },
        );
    }
    let record = LearnedSparsePromotionRecord {
        evaluation_id: "activation-mechanics-fixture-not-evaluation".to_string(),
        evaluation_date: "2026-10-03".to_string(),
        corpus_id: "activation-mechanics-fixture".to_string(),
        corpus_revision: "v1".to_string(),
        judgment_set_id: "activation-mechanics-fixture".to_string(),
        source_input_hash:
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        final_evaluation: true,
        class_final_real,
        judgment_set_hash: Some(ContentHash::new(
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        )?),
        environment: LearnedSparseEnvironment {
            operating_system: "linux".to_string(),
            architecture: "x86_64".to_string(),
            cpu_model: "in-memory activation mechanics fixture".to_string(),
            software_revision: "activation-mechanics-fixture".to_string(),
            warmup_policy: "not an evaluation; no benchmark warmups".to_string(),
            sample_count: 1,
        },
        data_fidelity: LearnedSparseDataFidelity::RealSillageTask,
        identity: LearnedSparseBenchmarkIdentity::from_sparse_identity(
            identity,
            "activation-fixture-v1",
        )?,
        route_configuration: LearnedSparseRouteConfiguration {
            route: LearnedSparseRoute::SparseFused,
            result_limit: 20,
            candidate_limit: 50,
            budget: SearchExecutionBudget::new(20, 50, 1_000, 0)?,
        },
        budgets,
        decisions,
        rollback_target: LearnedSparseRollbackTarget {
            route: LearnedSparseRoute::Hybrid,
            index_generation: IndexGenerationId::new(1),
        },
        report_hash: ContentHash::new(
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
        )?,
    };
    record.validate()?;
    Ok(record)
}

/// A real instance with one indexed document and an active sparse projection
/// bound to the fixture identity.
struct PreparedInstance {
    _temp: TempDir,
    layout: InstanceLayout,
    state: KernelState,
    manifest: InstanceManifest,
    identity: SparseIdentity,
    store: Arc<SqliteStore>,
    index: Arc<SqliteLearnedSparseIndex>,
    provider: Arc<InMemoryLearnedSparseProvider>,
}

fn sparse_profile_lines() -> &'static str {
    "sparse_enabled=true\n\
     sparse_endpoint=http://127.0.0.1:10002/v1/sparse\n\
     sparse_provider=splade-onnx\n\
     sparse_revision=762be6a7206e2f299182705972a65e5c46e62be2\n\
     sparse_artifact_hash=sha256:df924d41f0a18608bd0f6f27c4b0f411960b594b42267932201b90b766473a1a\n\
     sparse_preprocessing_version=splade-templates-trunc512-v1\n\
     sparse_model=prithivida/Splade_PP_en_v1\n\
     sparse_vocabulary_size=30522\n\
     sparse_term_cap=256\n\
     sparse_remote_provider=false\n\
     sparse_retention_policy=no_retention\n"
}

fn enable_sparse_profile(layout: &InstanceLayout) -> Result<(), Box<dyn std::error::Error>> {
    let mut contents = std::fs::read_to_string(&layout.manifest_path)?;
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(sparse_profile_lines());
    std::fs::write(&layout.manifest_path, contents)?;
    Ok(())
}

/// Indexes one real document through the runtime ingestion pipeline.
async fn index_shadow_store_document(
    layout: &InstanceLayout,
) -> Result<(), Box<dyn std::error::Error>> {
    let session =
        crate::MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace).await?;
    let bytes = b"bounded research observations are retained in the shadow-store ledger\n".to_vec();
    let artifact_id = sillage_core::artifact_id_for(&layout.root.join("shadow-store.md"), &bytes);
    let hash = ContentHash::new(sillage_domain::content_hash(&bytes))?;
    let result = async {
        session
            .submit(DomainInput::ArtifactDetected(
                sillage_domain::ArtifactDetected {
                    artifact_id,
                    title: "shadow-store.md".to_string(),
                    source_path: layout.root.join("shadow-store.md").display().to_string(),
                    source_bytes: bytes,
                    content_hash: hash,
                },
            ))
            .await?;
        // The parse runs as a runtime effect; wait for the durable
        // indexed state before the session drains and shuts down.
        wait_for_indexed(layout, artifact_id).await
    }
    .await;
    session.finish(result).await?;
    Ok(())
}

/// Materializes the full-text projection so the runtime can open it
/// read-only during the activation checks.
fn materialize_full_text_projection(
    layout: &InstanceLayout,
    state: &KernelState,
) -> Result<(), Box<dyn std::error::Error>> {
    let search_index = crate::projection_open::open_full_text_index(layout, state, true, false)?;
    crate::projection_recovery::reconcile_full_text_projection(state, &*search_index)?;
    drop(search_index);
    Ok(())
}

/// Registers the sparse generation with the fixture identity and activates it.
fn register_sparse_generation(
    layout: &InstanceLayout,
    state: &mut KernelState,
    store: &SqliteStore,
    identity: &SparseIdentity,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = layout;
    persist_input(
        state,
        store,
        DomainInput::StartIndexGeneration(StartIndexGenerationInput {
            id: identity.generation_id,
            name: RepresentationName::new(SPARSE_REPRESENTATION_V1),
            corpus_snapshot: identity.corpus_snapshot,
            fingerprint: fixture_index_fingerprint(identity),
            sparse_namespace: Some(identity.namespace.clone()),
        }),
    )?;
    advance_generation(state, store, identity.generation_id)?;
    Ok(())
}

async fn prepare() -> Result<PreparedInstance, Box<dyn std::error::Error>> {
    let temp = TempDir::create()?;
    let layout = crate::prepare_instance(temp.path().to_path_buf())?;
    enable_sparse_profile(&layout)?;
    let manifest = InstanceManifest::decode(&std::fs::read_to_string(&layout.manifest_path)?)?;

    // One real document indexed through the runtime ingestion pipeline.
    index_shadow_store_document(&layout).await?;
    let mut state = crate::load_kernel_state(&layout)?;
    materialize_full_text_projection(&layout, &state)?;

    // Register the sparse generation with the fixture identity and activate it.
    let store = Arc::new(SqliteStore::open(&layout.database_path)?);
    let identity = fixture_identity(IndexGenerationId::new(7))?;
    register_sparse_generation(&layout, &mut state, &store, &identity)?;

    // Populate and activate the SQLite projection with fixture vectors.
    let index = Arc::new(SqliteLearnedSparseIndex::new(
        store.clone(),
        identity.clone(),
    )?);
    let provider = Arc::new(InMemoryLearnedSparseProvider::new(identity.clone())?);
    let documents = state
        .chunks
        .values()
        .map(|chunk| {
            Ok(SparseDocument {
                chunk_id: chunk.id,
                content_hash: ContentHash::new(sillage_domain::content_hash(
                    chunk.text.as_bytes(),
                ))?,
                vector: provider.encode(
                    &chunk.text,
                    SparseInputKind::Document,
                    identity.clone(),
                )?,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    index.index_documents(documents)?;
    for next in [
        IndexLifecycle::Evaluated,
        IndexLifecycle::Shadow,
        IndexLifecycle::Active,
    ] {
        let current = index.lifecycle()?;
        index.transition(current, next)?;
    }
    Ok(PreparedInstance {
        _temp: temp,
        layout,
        state,
        manifest,
        identity,
        store,
        index,
        provider,
    })
}

async fn wait_for_indexed(
    layout: &InstanceLayout,
    artifact_id: sillage_domain::ArtifactId,
) -> Result<(), anyhow::Error> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow::anyhow!("timed out waiting for artifact indexing"));
        }
        let state = crate::load_kernel_state(layout)?;
        if state
            .artifacts
            .get(&artifact_id)
            .is_some_and(|artifact| artifact.index_status == IndexStatus::Indexed)
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn runtime_with(
    prepared: &PreparedInstance,
    policy: LearnedSparseExecutionPolicy,
    retriever: Option<Arc<dyn CandidateRetriever>>,
) -> Result<Arc<SearchRuntime>, Box<dyn std::error::Error>> {
    for directory in prepared.layout.required_directories() {
        std::fs::create_dir_all(&directory)?;
    }
    let search_index = crate::projection_open::open_full_text_index(
        &prepared.layout,
        &prepared.state,
        false,
        false,
    )?;
    let graph_index =
        crate::projection_open::open_graph_index(&prepared.layout, &prepared.state, false)?;
    let primary_generation = prepared
        .state
        .index_generations
        .get_active(&RepresentationName::new("lexical_text_v1"))
        .map(|generation| generation.id)
        .ok_or("primary lexical generation is missing")?;
    let runtime = SearchRuntime::from_parts(
        SearchRuntimeParts {
            artifacts: prepared.store.clone(),
            cards: prepared.store.clone(),
            chunks: prepared.store.clone(),
            evidence: prepared.store.clone(),
            search_index,
            blobs: Arc::new(sillage_blob_fs::FsBlobStore::open(
                &prepared.layout.blobs_dir,
            )?),
            vector_index: None,
            graph_index: Some(graph_index),
            event_log: prepared.store.clone(),
            primary_generation,
            dense_generation: None,
            repository_code_index: None,
            repository_execution_policy: sillage_retrieval::RepositoryExecutionPolicy::Shadow,
            hybrid_execution_policy: sillage_retrieval::HybridExecutionPolicy::Shadow,
            learned_sparse_execution_policy: policy,
            sparse_retriever: retriever,
            corpus_snapshot: sillage_domain::DEFAULT_CORPUS_SNAPSHOT_ID,
            scope_id: sillage_domain::DEFAULT_INSTANCE_SCOPE_ID,
        },
        None,
        sillage_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
    )?;
    Ok(Arc::new(runtime))
}

fn sparse_retriever(
    prepared: &PreparedInstance,
) -> Result<Arc<dyn CandidateRetriever>, Box<dyn std::error::Error>> {
    let capability = LearnedSparseGenerationCapability::activate(
        &prepared.state.index_generations,
        prepared.identity.clone(),
    )?;
    let retriever = LearnedSparseChunkRetriever::new(
        LearnedSparseChunkRetrieverParts {
            index: prepared.index.clone() as Arc<dyn LearnedSparseIndex + Send + Sync>,
            artifacts: prepared.store.clone(),
            chunks: prepared.store.clone(),
            evidence: prepared.store.clone(),
            blobs: Arc::new(sillage_blob_fs::FsBlobStore::open(
                &prepared.layout.blobs_dir,
            )?),
            provider: prepared.provider.clone(),
        },
        capability,
    )?;
    Ok(Arc::new(retriever))
}

fn has_sparse_scores(outcome: &sillage_domain::SearchOutcome) -> bool {
    outcome.evidence.iter().any(|candidate| {
        candidate
            .scores()
            .lane(&RetrievalScoreKind::LearnedSparse)
            .is_some()
    })
}

async fn search(
    runtime: &SearchRuntime,
    query: &str,
) -> Result<sillage_domain::SearchOutcome, Box<dyn std::error::Error>> {
    let engine = runtime.retrieval_engine()?;
    let plan = engine
        .plan(query, 10, &runtime.planner_context())
        .map_err(anyhow::Error::new)?;
    let plan = plan
        .confine_to_scope(runtime.scope_id)
        .map_err(anyhow::Error::new)?;
    let outcome = engine.search(&plan).map_err(anyhow::Error::new)?;
    Ok(outcome)
}

#[tokio::test]
async fn shadow_without_record_never_serves_sparse() -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare().await?;
    let runtime = runtime_with(
        &prepared,
        LearnedSparseExecutionPolicy::Shadow,
        Some(sparse_retriever(&prepared)?),
    )?;
    let outcome = search(&runtime, WINNING_QUERY).await?;
    assert!(
        !has_sparse_scores(&outcome),
        "shadow policy must not fuse learned-sparse scores"
    );
    let exact = search(&runtime, EXACT_QUERY).await?;
    assert!(!has_sparse_scores(&exact));
    assert!(!exact.evidence.is_empty());
    Ok(())
}

#[tokio::test]
async fn active_record_fuses_winning_class_and_protects_others()
-> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare().await?;
    let record = fixture_promotion_record(&prepared.identity)?;
    prepared.store.save_promotion_record(
        &record.corpus_id,
        &record.evaluation_id,
        &record.evaluation_date,
        record.report_hash.as_str(),
        &serde_json::to_string(&record)?,
    )?;

    let runtime = runtime_with(
        &prepared,
        LearnedSparseExecutionPolicy::Active(Box::new(record)),
        Some(sparse_retriever(&prepared)?),
    )?;
    let outcome = search(&runtime, WINNING_QUERY).await?;
    assert!(
        has_sparse_scores(&outcome),
        "the winning class must serve fused learned-sparse scores"
    );

    let exact = search(&runtime, EXACT_QUERY).await?;
    assert!(
        !has_sparse_scores(&exact),
        "ExactLiteral is protected: its route must stay lexical/exact"
    );
    let security = search(&runtime, SECURITY_QUERY).await?;
    assert!(
        !has_sparse_scores(&security),
        "Security is protected: its route must stay hybrid"
    );
    Ok(())
}

#[tokio::test]
async fn removing_the_record_restores_the_shadow_trace() -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare().await?;
    let shadow_runtime = runtime_with(
        &prepared,
        LearnedSparseExecutionPolicy::Shadow,
        Some(sparse_retriever(&prepared)?),
    )?;
    let shadow_outcome = search(&shadow_runtime, WINNING_QUERY).await?;

    let record = fixture_promotion_record(&prepared.identity)?;
    prepared.store.save_promotion_record(
        &record.corpus_id,
        &record.evaluation_id,
        &record.evaluation_date,
        record.report_hash.as_str(),
        &serde_json::to_string(&record)?,
    )?;
    let active_runtime = runtime_with(
        &prepared,
        LearnedSparseExecutionPolicy::Active(Box::new(record)),
        Some(sparse_retriever(&prepared)?),
    )?;
    let active_outcome = search(&active_runtime, WINNING_QUERY).await?;
    assert!(has_sparse_scores(&active_outcome));

    prepared.store.remove_all_promotion_records()?;
    let restored_runtime = runtime_with(
        &prepared,
        LearnedSparseExecutionPolicy::Shadow,
        Some(sparse_retriever(&prepared)?),
    )?;
    let restored_outcome = search(&restored_runtime, WINNING_QUERY).await?;
    assert!(!has_sparse_scores(&restored_outcome));
    assert_eq!(shadow_outcome.evidence, restored_outcome.evidence);
    Ok(())
}

#[tokio::test]
async fn invalid_record_stays_shadowed() -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare().await?;
    let mut record = fixture_promotion_record(&prepared.identity)?;
    record.final_evaluation = false;
    prepared.store.save_promotion_record(
        &record.corpus_id,
        &record.evaluation_id,
        &record.evaluation_date,
        record.report_hash.as_str(),
        &serde_json::to_string(&record)?,
    )?;
    let stored = prepared
        .store
        .load_latest_promotion_record()?
        .ok_or("record was not stored")?;
    let parsed: LearnedSparsePromotionRecord = serde_json::from_str(&stored.record_json)?;
    assert!(parsed.validate().is_err());

    let policy =
        crate::runtime_construction::learned_sparse_policy(&prepared.store, &prepared.manifest);
    assert_eq!(policy, LearnedSparseExecutionPolicy::Shadow);
    let runtime = runtime_with(&prepared, policy, Some(sparse_retriever(&prepared)?))?;
    let outcome = search(&runtime, WINNING_QUERY).await?;
    assert!(!has_sparse_scores(&outcome));
    Ok(())
}

#[tokio::test]
async fn rolled_back_generation_degrades_to_hybrid() -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare().await?;
    let record = fixture_promotion_record(&prepared.identity)?;
    prepared.store.save_promotion_record(
        &record.corpus_id,
        &record.evaluation_id,
        &record.evaluation_date,
        record.report_hash.as_str(),
        &serde_json::to_string(&record)?,
    )?;

    // Roll the sparse generation back in the registry while the record
    // exists: retriever construction must fail and the lane degrade.
    let mut state = prepared.state.clone();
    persist_input(
        &mut state,
        &prepared.store,
        DomainInput::TransitionIndexGeneration(sillage_domain::TransitionIndexGenerationInput {
            id: prepared.identity.generation_id,
            to: IndexLifecycle::Retired,
        }),
    )?;

    let degraded = crate::runtime_construction::build_sparse_retriever(
        &state,
        &prepared.manifest,
        prepared.store.clone(),
        Arc::new(sillage_blob_fs::FsBlobStore::open(
            &prepared.layout.blobs_dir,
        )?),
    );
    assert!(
        degraded.is_none(),
        "a rolled-back generation must not construct a sparse retriever"
    );
    let runtime = runtime_with(
        &prepared,
        LearnedSparseExecutionPolicy::Active(Box::new(record)),
        None,
    )?;
    let outcome = search(&runtime, WINNING_QUERY).await?;
    assert!(
        !has_sparse_scores(&outcome),
        "the degraded lane must serve hybrid without sparse scores"
    );
    Ok(())
}
