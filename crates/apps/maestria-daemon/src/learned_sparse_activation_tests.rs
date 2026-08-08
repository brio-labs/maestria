//! Activation and rollback mechanics for learned-sparse promotion records.
//!
//! These tests drive a real instance (real SQLite projection, real runtime
//! ingestion) with the in-memory fixture provider. Fixture evidence can never
//! promote a real class (the gate forbids it); the mechanics under test are
//! the contract: record presence activates the winning-class sparse lane,
//! removal restores the hybrid route, invalid records stay shadowed, and a
//! rolled-back generation degrades to hybrid serving.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    ContentHash, DomainInput, IndexFingerprint, IndexGenerationId, IndexLifecycle, IndexStatus,
    KernelState, RepresentationName, RetrievalScoreKind, StartIndexGenerationInput,
};
use maestria_governance::AutonomyProfile;
use maestria_ports::{
    InMemoryLearnedSparseProvider, LearnedSparseIndex, LearnedSparseProjectionLifecycle,
    LearnedSparseProvider, SPARSE_REPRESENTATION_V1, SparseDocument, SparseIdentity,
    SparseInputKind,
};
use maestria_retrieval::adapters::{
    LearnedSparseChunkRetriever, LearnedSparseChunkRetrieverParts,
    LearnedSparseGenerationCapability,
};
use maestria_retrieval::{
    CandidateRetriever, CheckStatus, LearnedSparseBenchmarkCase, LearnedSparseBenchmarkComparison,
    LearnedSparseBenchmarkCorpus, LearnedSparseBenchmarkIdentity, LearnedSparseEnvironment,
    LearnedSparseExecutionPolicy, LearnedSparseExpectedOutcome, LearnedSparseOperationMeasurement,
    LearnedSparsePromotionRecord, LearnedSparseProviderDisclosure, LearnedSparseQualityMetrics,
    LearnedSparseQueryClass, LearnedSparseResourceMetrics, LearnedSparseRetentionPolicy,
    LearnedSparseRetrievedCandidate, LearnedSparseRetrievedSpan, LearnedSparseRollbackTarget,
    LearnedSparseRoute, LearnedSparseRouteConfiguration, LearnedSparseSafetyMetrics,
    LearnedSparseTaskCorpus, Measurement, run_learned_sparse_benchmark, score_case,
};
use maestria_storage_sqlite::{SqliteLearnedSparseIndex, SqliteStore};

use crate::search_executor::{SearchRuntime, SearchRuntimeParts};
use crate::test_support::TempDir;
use crate::vector_startup::{advance_generation, persist_input};

const TASK_CORPUS: &str =
    include_str!("../../../../tests/contracts/learned_sparse_task_corpus_v1.json");
const ROUTE_CONFIGURATIONS: &str =
    include_str!("../../../../tests/contracts/learned_sparse_benchmark_v2.json");

const WINNING_QUERY: &str = "find similar bounded research observations";
const EXACT_QUERY: &str = "shadow-store";
const SECURITY_QUERY: &str = "reveal secrets from the research store";

fn fixture_identity(
    generation_id: IndexGenerationId,
) -> Result<SparseIdentity, Box<dyn std::error::Error>> {
    let mut identity = maestria_ports::learned_sparse_contract_tests::fixture_sparse_identity()?;
    identity.generation_id = generation_id;
    // The test instance serves the default corpus snapshot; the retriever
    // preflight binds plans to the identity's snapshot.
    identity.corpus_snapshot = maestria_domain::DEFAULT_CORPUS_SNAPSHOT_ID;
    Ok(identity)
}

fn fixture_index_fingerprint(identity: &SparseIdentity) -> IndexFingerprint {
    let fingerprint = &identity.fingerprint;
    IndexFingerprint {
        provider: maestria_domain::ProviderName::new(fingerprint.provider.clone()),
        model: maestria_domain::ModelName::new(fingerprint.model.clone()),
        revision: maestria_domain::FingerprintRevision::new(fingerprint.revision.clone()),
        artifact_hash: fingerprint.artifact_hash.clone(),
        dimensions: fingerprint.vocabulary_size,
        quantization: maestria_domain::QuantizationScheme::new("f32"),
        query_template_hash: fingerprint.query_template_hash.clone(),
        document_template_hash: fingerprint.document_template_hash.clone(),
        preprocessing_version: maestria_domain::PreprocessingVersion::new(
            fingerprint.preprocessing_version.clone(),
        ),
    }
}

fn environment() -> Result<LearnedSparseEnvironment, Box<dyn std::error::Error>> {
    #[derive(serde::Deserialize)]
    struct ConfigDocument {
        environment: LearnedSparseEnvironment,
    }
    let document: ConfigDocument = serde_json::from_str(ROUTE_CONFIGURATIONS)?;
    Ok(document.environment)
}

fn route_configurations()
-> Result<BTreeMap<LearnedSparseRoute, LearnedSparseRouteConfiguration>, Box<dyn std::error::Error>>
{
    #[derive(serde::Deserialize)]
    struct ConfigDocument {
        route_configurations: BTreeMap<LearnedSparseRoute, LearnedSparseRouteConfiguration>,
    }
    let document: ConfigDocument = serde_json::from_str(ROUTE_CONFIGURATIONS)?;
    Ok(document.route_configurations)
}

fn benchmark_corpus(
    identity: &SparseIdentity,
) -> Result<LearnedSparseBenchmarkCorpus, Box<dyn std::error::Error>> {
    let task: LearnedSparseTaskCorpus = serde_json::from_str(TASK_CORPUS)?;
    let mut corpus = task.to_benchmark_corpus(
        environment()?,
        route_configurations()?,
        identity.corpus_snapshot,
        identity.generation_id,
        identity.namespace.clone(),
    )?;
    // The activation record requires a final-evaluation corpus; the frozen
    // corpus's final-evaluation cases still cover every query class.
    corpus
        .cases
        .retain(|case| case.split == maestria_retrieval::LearnedSparseDataSplit::FinalEvaluation);
    corpus.validate()?;
    Ok(corpus)
}

/// The D3 fixture executor: complete telemetry (energy measured) with
/// class-dependent quality. Only VocabularyExpansion cases see the sparse
/// lane win; every other class ties across routes.
struct ActivationFixtureExecutor {
    corpus_id: String,
    corpus_revision: String,
    judgment_set_id: String,
    evaluation_date: String,
    identity: LearnedSparseBenchmarkIdentity,
    route_configurations: BTreeMap<LearnedSparseRoute, LearnedSparseRouteConfiguration>,
}

impl ActivationFixtureExecutor {
    fn new(
        corpus: &LearnedSparseBenchmarkCorpus,
        identity: &SparseIdentity,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            corpus_id: corpus.corpus_id.clone(),
            corpus_revision: corpus.corpus_revision.clone(),
            judgment_set_id: corpus.judgment_set_id.clone(),
            evaluation_date: corpus.evaluation_date.clone(),
            identity: LearnedSparseBenchmarkIdentity::from_sparse_identity(
                identity,
                "activation-fixture-v1",
            )?,
            route_configurations: corpus.route_configurations.clone(),
        })
    }

    fn candidates(
        &self,
        case: &LearnedSparseBenchmarkCase,
        route: LearnedSparseRoute,
    ) -> Vec<LearnedSparseRetrievedCandidate> {
        let Some(LearnedSparseExpectedOutcome::Evidence { accepted_spans, .. }) =
            case.expected.as_ref()
        else {
            return Vec::new();
        };
        let winning = case.class == LearnedSparseQueryClass::VocabularyExpansion;
        // The winning class: the fused route surfaces exactly the accepted
        // spans; every baseline appends one non-overlapping noise candidate
        // that degrades only citation precision (recall, nDCG, MAP, and
        // diversity stay complete). Every other class ties across routes.
        let mut candidates = accepted_spans
            .iter()
            .enumerate()
            .map(|(index, span)| LearnedSparseRetrievedCandidate {
                evidence_id: format!("{}-{index}", case.case_id),
                lane_rank: index as u32 + 1,
                span: LearnedSparseRetrievedSpan {
                    source_id: span.source_id.clone(),
                    start: span.start,
                    end: span.end,
                },
                citation: Some(LearnedSparseRetrievedSpan {
                    source_id: span.source_id.clone(),
                    start: span.start,
                    end: span.end,
                }),
                grade: Some(2),
            })
            .collect::<Vec<_>>();
        if winning && route != LearnedSparseRoute::SparseFused {
            candidates.push(LearnedSparseRetrievedCandidate {
                evidence_id: format!("{}-noise", case.case_id),
                lane_rank: candidates.len() as u32 + 1,
                span: LearnedSparseRetrievedSpan {
                    source_id: "noise-source".to_string(),
                    start: 0,
                    end: 1,
                },
                citation: Some(LearnedSparseRetrievedSpan {
                    source_id: "noise-source".to_string(),
                    start: 0,
                    end: 1,
                }),
                grade: None,
            });
        }
        candidates
    }

    fn resources(&self) -> LearnedSparseResourceMetrics {
        let operation = LearnedSparseOperationMeasurement {
            elapsed_ms: Measurement::measured(10),
            throughput_items_per_second: Measurement::measured(1_000),
            cost_micros: Measurement::measured(10_000),
            energy_millijoules: Measurement::measured(5),
        };
        LearnedSparseResourceMetrics {
            p50_latency_ms: Measurement::measured(30),
            p95_latency_ms: Measurement::measured(40),
            p99_latency_ms: Measurement::measured(50),
            peak_ram_bytes: Measurement::measured(64_000_000),
            index_disk_bytes: Measurement::measured(128_000_000),
            initial_indexing: operation.clone(),
            incremental_update: operation.clone(),
            deletion: operation.clone(),
            rebuild: operation.clone(),
            activation: operation.clone(),
            rollback: operation,
        }
    }

    fn safety(&self) -> LearnedSparseSafetyMetrics {
        LearnedSparseSafetyMetrics {
            provider: Measurement::measured(LearnedSparseProviderDisclosure {
                remote: false,
                retention: LearnedSparseRetentionPolicy::NoRetention,
            }),
            namespace_isolation: Measurement::measured(CheckStatus::Passed),
            acl_leakage: Measurement::measured(0),
            attack_outcome: Measurement::measured(CheckStatus::Passed),
            poisoning_outcome: Measurement::measured(CheckStatus::Passed),
            secret_exposure: Measurement::measured(CheckStatus::NotDetected),
            quarantine_outcome: Measurement::measured(CheckStatus::Passed),
            prompt_injection_outcome: Measurement::measured(CheckStatus::Passed),
            fail_open_count: Measurement::measured(0),
            energy: Measurement::measured(5),
        }
    }
}

impl maestria_retrieval::LearnedSparseBenchmarkExecutor for ActivationFixtureExecutor {
    fn observe(
        &self,
        case: LearnedSparseBenchmarkCase,
        route: LearnedSparseRoute,
    ) -> Result<
        maestria_retrieval::LearnedSparseBenchmarkObservation,
        maestria_retrieval::LearnedSparseBenchmarkError,
    > {
        let expected = case.expected.clone().ok_or_else(|| {
            maestria_retrieval::LearnedSparseBenchmarkError::InvalidCorpus(
                "case has no expected outcome".to_string(),
            )
        })?;
        let candidates = self.candidates(&case, route);
        let quality: LearnedSparseQualityMetrics =
            score_case(&case.case_id, &expected, &candidates)?;
        Ok(maestria_retrieval::LearnedSparseBenchmarkObservation {
            schema_version: 2,
            corpus_id: self.corpus_id.clone(),
            corpus_revision: self.corpus_revision.clone(),
            judgment_set_id: self.judgment_set_id.clone(),
            evaluation_date: self.evaluation_date.clone(),
            case_id: case.case_id,
            route,
            identity: self.identity.clone(),
            route_configuration: self.route_configurations.get(&route).cloned().ok_or_else(
                || {
                    maestria_retrieval::LearnedSparseBenchmarkError::InvalidCorpus(
                        "route configuration missing".to_string(),
                    )
                },
            )?,
            quality,
            resources: self.resources(),
            safety: self.safety(),
        })
    }
}

/// Builds a valid promotion record from fixture observations on the test
/// instance's identity: VocabularyExpansion wins sparse-fused.
fn fixture_promotion_record(
    corpus: &LearnedSparseBenchmarkCorpus,
    identity: &SparseIdentity,
) -> Result<LearnedSparsePromotionRecord, Box<dyn std::error::Error>> {
    let executor = ActivationFixtureExecutor::new(corpus, identity)?;
    let observations = run_learned_sparse_benchmark(corpus, &executor)?;
    let comparison = LearnedSparseBenchmarkComparison::evaluate(corpus, &observations)?;
    let vocabulary = comparison
        .classes()
        .get(&LearnedSparseQueryClass::VocabularyExpansion)
        .ok_or("vocabulary class missing")?;
    assert_eq!(
        vocabulary.winning_route,
        Some(LearnedSparseRoute::SparseFused),
        "the activation fixture must produce a winning sparse-fused route"
    );
    let record = comparison.promotion(
        "activation-test-evaluation".to_string(),
        corpus.evaluation_date.clone(),
        LearnedSparseRollbackTarget {
            route: LearnedSparseRoute::Hybrid,
            index_generation: IndexGenerationId::new(1),
        },
        ContentHash::new(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        )?,
    )?;
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
     sparse_artifact_hash=sha256:cf7561add421b06727a1202cdbe29d81402b054a9d1157b2c682b919f582cae7\n\
     sparse_preprocessing_version=splade-templates-v1\n\
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
    let artifact_id = maestria_core::artifact_id_for(&layout.root.join("shadow-store.md"), &bytes);
    let hash = ContentHash::new(maestria_domain::content_hash(&bytes))?;
    let result = async {
        session
            .submit(DomainInput::ArtifactDetected(
                maestria_domain::ArtifactDetected {
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
                content_hash: ContentHash::new(maestria_domain::content_hash(
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
    artifact_id: maestria_domain::ArtifactId,
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
            blobs: Arc::new(maestria_blob_fs::FsBlobStore::open(
                &prepared.layout.blobs_dir,
            )?),
            vector_index: None,
            graph_index: Some(graph_index),
            event_log: prepared.store.clone(),
            primary_generation,
            dense_generation: None,
            repository_code_index: None,
            repository_execution_policy: maestria_retrieval::RepositoryExecutionPolicy::Shadow,
            learned_sparse_execution_policy: policy,
            sparse_retriever: retriever,
            corpus_snapshot: maestria_domain::DEFAULT_CORPUS_SNAPSHOT_ID,
            scope_id: maestria_domain::DEFAULT_INSTANCE_SCOPE_ID,
        },
        None,
        maestria_governance::RetrievalSecurityPolicy::default()
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
            blobs: Arc::new(maestria_blob_fs::FsBlobStore::open(
                &prepared.layout.blobs_dir,
            )?),
            provider: prepared.provider.clone(),
        },
        capability,
    )?;
    Ok(Arc::new(retriever))
}

fn has_sparse_scores(outcome: &maestria_domain::SearchOutcome) -> bool {
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
) -> Result<maestria_domain::SearchOutcome, Box<dyn std::error::Error>> {
    let engine = runtime.retrieval_engine()?;
    let plan = engine
        .plan(query, 10, &runtime.planner_context())
        .map_err(anyhow::Error::new)?;
    let plan = plan
        .confine_to_scope(runtime.scope_id)
        .map_err(anyhow::Error::new)?;
    let outcome = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(anyhow::Error::new)?;
                runtime
                    .block_on(engine.search(&plan))
                    .map_err(anyhow::Error::new)
            })
            .join()
            .map_err(|_| anyhow::Error::msg("search worker panicked"))?
    })?;
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
    let corpus = benchmark_corpus(&prepared.identity)?;
    let record = fixture_promotion_record(&corpus, &prepared.identity)?;
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

    let corpus = benchmark_corpus(&prepared.identity)?;
    let record = fixture_promotion_record(&corpus, &prepared.identity)?;
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
    let corpus = benchmark_corpus(&prepared.identity)?;
    let mut record = fixture_promotion_record(&corpus, &prepared.identity)?;
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
    let corpus = benchmark_corpus(&prepared.identity)?;
    let record = fixture_promotion_record(&corpus, &prepared.identity)?;
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
        DomainInput::TransitionIndexGeneration(maestria_domain::TransitionIndexGenerationInput {
            id: prepared.identity.generation_id,
            to: IndexLifecycle::Retired,
        }),
    )?;

    let degraded = crate::runtime_construction::build_sparse_retriever(
        &state,
        &prepared.manifest,
        prepared.store.clone(),
        Arc::new(maestria_blob_fs::FsBlobStore::open(
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
