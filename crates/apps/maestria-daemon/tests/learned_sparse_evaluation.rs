//! Real four-profile learned-sparse evaluation (manual, never CI).
//!
//! Requires `MAESTRIA_SPARSE_EVALUATION=1`, a prepared instance indexed from
//! the frozen corpus source inputs, and the pinned SPLADE sidecar running on
//! 127.0.0.1:10002. The run prints per-class decisions and writes the dated
//! report for the evidence ledger. If provider/budget telemetry is
//! unavailable the gate yields no promotion; that is a valid completion.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use maestria_core::{artifact_id_for, content_hash};
use maestria_domain::{ContentHash, DomainInput, KernelState};
use maestria_governance::AutonomyProfile;
use maestria_retrieval::{
    LearnedSparseBenchmarkComparison, LearnedSparseBenchmarkCorpus, LearnedSparseClassDecision,
    LearnedSparseEnvironment, LearnedSparseQueryClass, LearnedSparseRollbackTarget,
    LearnedSparseRoute, LearnedSparseRouteConfiguration, LearnedSparseTaskCorpus,
    run_learned_sparse_benchmark,
};

use maestria_daemon::{
    LearnedSparseBenchmarkExecutor, MutationSession, load_kernel_state,
    prepare_instance_with_roots, reconcile_sparse_generation, sparse_namespace,
};

const TASK_CORPUS: &str =
    include_str!("../../../../tests/contracts/learned_sparse_task_corpus_v1.json");
const ROUTE_CONFIGURATIONS: &str =
    include_str!("../../../../tests/contracts/learned_sparse_benchmark_v2.json");

const SPARSE_PROFILE: &[(&str, &str)] = &[
    ("sparse_enabled", "true"),
    ("sparse_endpoint", "http://127.0.0.1:10002/v1/sparse"),
    ("sparse_provider", "splade-onnx"),
    (
        "sparse_revision",
        "762be6a7206e2f299182705972a65e5c46e62be2",
    ),
    (
        "sparse_artifact_hash",
        "sha256:cf7561add421b06727a1202cdbe29d81402b054a9d1157b2c682b919f582cae7",
    ),
    ("sparse_preprocessing_version", "splade-templates-v1"),
    ("sparse_model", "prithivida/Splade_PP_en_v1"),
    ("sparse_vocabulary_size", "30522"),
    ("sparse_term_cap", "256"),
    ("sparse_remote_provider", "false"),
    ("sparse_retention_policy", "no_retention"),
];

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn create() -> std::io::Result<Self> {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "maestria-sparse-evaluation-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repo_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    Ok(root.canonicalize()?)
}

fn task_corpus() -> Result<LearnedSparseTaskCorpus, Box<dyn std::error::Error>> {
    let corpus: LearnedSparseTaskCorpus = serde_json::from_str(TASK_CORPUS)?;
    corpus.validate()?;
    Ok(corpus)
}

fn environment() -> Result<LearnedSparseEnvironment, Box<dyn std::error::Error>> {
    #[derive(serde::Deserialize)]
    struct ConfigDocument {
        environment: LearnedSparseEnvironment,
        route_configurations: BTreeMap<LearnedSparseRoute, LearnedSparseRouteConfiguration>,
    }
    let document: ConfigDocument = serde_json::from_str(ROUTE_CONFIGURATIONS)?;
    let _ = document.route_configurations;
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

/// Enables the pinned sparse profile on the instance manifest.
fn enable_sparse_profile(
    layout: &maestria_core::InstanceLayout,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut contents = fs::read_to_string(&layout.manifest_path)?;
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    for (key, value) in SPARSE_PROFILE {
        contents.push_str(&format!("{key}={value}\n"));
    }
    fs::write(&layout.manifest_path, contents)?;
    Ok(())
}

/// The commit whose tree content matches every frozen corpus source hash.
///
/// The live working tree has drifted since the corpus freeze; the evaluation
/// runs against the exact frozen content so the accepted spans and evidence
/// remain the dated evidence the judgments describe.
const CORPUS_CONTENT_COMMIT: &str = "658d1af1";

/// Materializes the frozen corpus sources under `sources_dir` and verifies
/// every content hash. Returns the map of materialized path to source id.
fn materialize_sources(
    corpus: &LearnedSparseTaskCorpus,
    root: &Path,
    sources_dir: &Path,
) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
    let mut source_ids = BTreeMap::new();
    for source in &corpus.source_inputs {
        let output = std::process::Command::new("git")
            .arg("show")
            .arg(format!("{CORPUS_CONTENT_COMMIT}:{}", source.path))
            .current_dir(root)
            .output()
            .map_err(|error| format!("git show {}: {error}", source.path))?;
        if !output.status.success() {
            return Err(format!(
                "git show {} failed: {}",
                source.path,
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        let actual = content_hash(&output.stdout);
        if actual != source.content_hash.as_str() {
            return Err(format!(
                "corpus source {} hash drift at {CORPUS_CONTENT_COMMIT}: expected {}, got {actual}",
                source.path,
                source.content_hash.as_str()
            )
            .into());
        }
        let destination = sources_dir.join(&source.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, &output.stdout)?;
        source_ids.insert(destination.display().to_string(), source.source_id.clone());
    }
    Ok(source_ids)
}

/// Polls persisted kernel state until the artifact reaches `Indexed`.
async fn wait_for_indexed(
    layout: &maestria_core::InstanceLayout,
    artifact_id: maestria_domain::ArtifactId,
) -> anyhow::Result<()> {
    let budget = std::time::Duration::from_secs(120);
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow::anyhow!("timed out waiting for artifact indexing"));
        }
        match maestria_daemon::load_kernel_state(layout) {
            Ok(state)
                if state.artifacts.get(&artifact_id).is_some_and(|artifact| {
                    artifact.index_status == maestria_domain::IndexStatus::Indexed
                }) =>
            {
                return Ok(());
            }
            Ok(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
            Err(error) if maestria_daemon::db_retry::is_database_busy(&error) => {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Indexes one corpus source through the real runtime ingestion pipeline.
async fn index_source(
    session: &MutationSession,
    layout: &maestria_core::InstanceLayout,
    path: &Path,
) -> anyhow::Result<()> {
    let bytes = fs::read(path)?;
    let artifact_id = artifact_id_for(path, &bytes);
    let hash = ContentHash::new(content_hash(&bytes))?;
    session
        .submit(DomainInput::ArtifactDetected(
            maestria_domain::ArtifactDetected {
                artifact_id,
                title: match path.file_name().and_then(|name| name.to_str()) {
                    Some(name) => name.to_string(),
                    None => "source".to_string(),
                },
                source_path: path.display().to_string(),
                source_bytes: bytes,
                content_hash: hash,
            },
        ))
        .await?;
    wait_for_indexed(layout, artifact_id).await
}

fn index_generation_label(generation: u64) -> String {
    format!("sparse-text-v1-{generation}")
}

fn model_fingerprint(identity: &maestria_ports::SparseIdentity) -> String {
    let fingerprint = &identity.fingerprint;
    format!(
        "sparse:{}:{}:{}:{}:{}",
        fingerprint.provider,
        fingerprint.model,
        fingerprint.revision,
        fingerprint.vocabulary_hash.as_str(),
        fingerprint.preprocessing_version
    )
}

fn decisions_from_comparison(
    comparison: &LearnedSparseBenchmarkComparison,
) -> BTreeMap<LearnedSparseQueryClass, LearnedSparseClassDecision> {
    comparison
        .classes()
        .iter()
        .map(|(class, entry)| {
            let decision = match entry.winning_route {
                Some(LearnedSparseRoute::SparseFused) => {
                    LearnedSparseClassDecision::PromoteSparseFused
                }
                _ if matches!(
                    class,
                    LearnedSparseQueryClass::ExactLiteral
                        | LearnedSparseQueryClass::NoEvidence
                        | LearnedSparseQueryClass::Security
                ) =>
                {
                    LearnedSparseClassDecision::RetainLexical
                }
                _ => LearnedSparseClassDecision::RetainHybrid,
            };
            (*class, decision)
        })
        .collect()
}

#[tokio::test]
#[ignore = "requires the pinned SPLADE sidecar and a prepared real instance"]
async fn learned_sparse_four_profile_real_evaluation() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("MAESTRIA_SPARSE_EVALUATION").as_deref() != Ok("1") {
        eprintln!("skipping: MAESTRIA_SPARSE_EVALUATION=1 is required");
        return Ok(());
    }
    let root = repo_root()?;
    let temp = TempDir::create()?;
    let prepared = prepare_evaluation_instance(&root, temp.path()).await?;
    let (_layout, state, _manifest, benchmark_corpus, executor, sparse_generation) = prepared;
    let _ = state;

    let observations = run_learned_sparse_benchmark(&benchmark_corpus, &executor)?;
    let comparison = LearnedSparseBenchmarkComparison::evaluate(&benchmark_corpus, &observations)?;
    let decisions = decisions_from_comparison(&comparison);
    println!("== per-class decisions (winning route) ==");
    for (class, entry) in comparison.classes() {
        println!("{class:?}: winning_route={:?}", entry.winning_route);
    }
    let won = comparison
        .classes()
        .values()
        .any(|entry| entry.winning_route == Some(LearnedSparseRoute::SparseFused));

    // Report emission: the dated report is the serialized comparison plus the
    // ledger-bound identity fields, mirroring the contract report shape.
    let report_path = emit_report(
        &root,
        &executor,
        &benchmark_corpus,
        &observations,
        sparse_generation,
        decisions,
    )?;
    let report_hash = ContentHash::new(content_hash(&fs::read(&report_path)?))?;
    println!(
        "report written: {} (sha256 {})",
        report_path.display(),
        report_hash.as_str()
    );
    let identity = executor
        .sparse_identity_for_report()
        .ok_or("sparse identity is unavailable for the report")?;
    println!(
        "ledger fingerprints: index_generation={} model_fingerprint={}",
        index_generation_label(sparse_generation),
        model_fingerprint(&identity)
    );

    // The gate decision: a valid promotion record when at least one eligible
    // class won with complete telemetry; otherwise the honest no-promotion.
    let promotion = comparison.promotion(
        "learned-sparse-four-profile-2026-08-07".to_string(),
        benchmark_corpus.evaluation_date.clone(),
        LearnedSparseRollbackTarget {
            route: LearnedSparseRoute::Hybrid,
            index_generation: state
                .index_generations
                .get_active(&maestria_domain::RepresentationName::new("lexical_text_v1"))
                .map(|generation| generation.id)
                .ok_or("primary lexical generation is missing")?,
        },
        report_hash,
    )?;
    if won {
        assert!(
            promotion.validate().is_ok(),
            "winning classes must produce a valid promotion record"
        );
        println!("== promotion record ==");
        println!("{}", serde_json::to_string_pretty(&promotion)?);
    } else {
        assert!(
            promotion.validate().is_err(),
            "no winning class must not produce a promotion record"
        );
        println!("== no class won; no promotion record is produced ==");
    }
    Ok(())
}

/// Writes the dated real-evaluation report and returns its path.
fn emit_report(
    root: &Path,
    executor: &LearnedSparseBenchmarkExecutor,
    corpus: &LearnedSparseBenchmarkCorpus,
    observations: &[maestria_retrieval::LearnedSparseBenchmarkObservation],
    sparse_generation: u64,
    decisions: BTreeMap<LearnedSparseQueryClass, LearnedSparseClassDecision>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[derive(serde::Serialize)]
    struct ReportObservation<'a> {
        case_id: &'a str,
        route: LearnedSparseRoute,
        quality: &'a maestria_retrieval::LearnedSparseQualityMetrics,
        resources: &'a maestria_retrieval::LearnedSparseResourceMetrics,
        safety: &'a maestria_retrieval::LearnedSparseSafetyMetrics,
        measurement_status: &'static str,
    }
    #[derive(serde::Serialize)]
    struct Report<'a> {
        measurement_kind: &'static str,
        evaluation_date: &'a str,
        corpus_id: &'a str,
        corpus_revision: &'a str,
        index_generation: String,
        model_fingerprint: String,
        namespace: String,
        route_configuration: &'a LearnedSparseRouteConfiguration,
        observations: Vec<ReportObservation<'a>>,
        decisions: BTreeMap<LearnedSparseQueryClass, LearnedSparseClassDecision>,
    }
    let report_dir = match std::env::var("MAESTRIA_BENCHMARK_REPORT_DIR") {
        Ok(report_dir) => report_dir,
        Err(_) => root.join("target/benchmark-reports").display().to_string(),
    };
    fs::create_dir_all(&report_dir)?;
    let identity = executor
        .sparse_identity_for_report()
        .ok_or("sparse identity is unavailable for the report")?;
    let route_configuration = corpus
        .route_configurations
        .get(&LearnedSparseRoute::SparseFused)
        .cloned()
        .ok_or("sparse-fused route configuration is missing")?;
    let report_path = Path::new(&report_dir).join("learned-sparse.json");
    let report = Report {
        measurement_kind: "learned_sparse_four_profile",
        evaluation_date: &corpus.evaluation_date,
        corpus_id: &corpus.corpus_id,
        corpus_revision: &corpus.corpus_revision,
        index_generation: index_generation_label(sparse_generation),
        model_fingerprint: model_fingerprint(&identity),
        namespace: format!(
            "{}:{:?}:{}",
            identity.namespace.instance_id(),
            identity.namespace.trust_zone(),
            identity.namespace.projection()
        ),
        route_configuration: &route_configuration,
        observations: observations
            .iter()
            .map(|observation| ReportObservation {
                case_id: &observation.case_id,
                route: observation.route,
                quality: &observation.quality,
                resources: &observation.resources,
                safety: &observation.safety,
                measurement_status: "Measured",
            })
            .collect(),
        decisions,
    };
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    Ok(report_path)
}

/// Prepares the evaluation instance: indexes the frozen sources, binds the
/// benchmark corpus, and builds the four-profile executor.
async fn prepare_evaluation_instance(
    root: &Path,
    instance_dir: &Path,
) -> Result<
    (
        maestria_core::InstanceLayout,
        KernelState,
        maestria_core::InstanceManifest,
        LearnedSparseBenchmarkCorpus,
        LearnedSparseBenchmarkExecutor,
        u64,
    ),
    Box<dyn std::error::Error>,
> {
    let sources_dir = instance_dir.join("sources");
    fs::create_dir_all(&sources_dir)?;
    let layout =
        prepare_instance_with_roots(instance_dir.to_path_buf(), vec![sources_dir.clone()])?;
    enable_sparse_profile(&layout)?;

    // C1: index the frozen corpus source inputs with verified hashes. The
    // content is the dated freeze, not the drifted working tree.
    let corpus = task_corpus()?;
    let source_ids = materialize_sources(&corpus, root, &sources_dir)?;
    {
        let session =
            MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace).await?;
        let result = async {
            for path in source_ids.keys() {
                index_source(&session, &layout, Path::new(path)).await?;
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        session.finish(result).await?;
    }

    let mut state: KernelState = load_kernel_state(&layout)?;
    let manifest = maestria_core::InstanceService::parse_manifest(&fs::read_to_string(
        &layout.manifest_path,
    )?)?;
    let generation_id = reconcile_sparse_generation(&layout, &mut state, &manifest)?;
    let namespace = sparse_namespace(&manifest)?;

    // C2: the benchmark corpus bound to this instance.
    let snapshot = state
        .index_generations
        .get(generation_id)
        .map(|generation| generation.corpus_snapshot)
        .ok_or("sparse generation disappeared")?;
    let benchmark_corpus = corpus.to_benchmark_corpus(
        environment()?,
        route_configurations()?,
        snapshot,
        generation_id,
        namespace,
    )?;

    let chunks = state.chunks.values().cloned().collect::<Vec<_>>();
    if chunks.is_empty() {
        return Err("the prepared instance has no indexed chunks".into());
    }
    let executor = LearnedSparseBenchmarkExecutor::prepare(
        &layout,
        &mut state,
        &manifest,
        benchmark_corpus.clone(),
        source_ids,
        chunks,
    )?;
    let sparse_generation = executor
        .sparse_generation_id()
        .ok_or("sparse generation missing")?
        .value();
    Ok((
        layout,
        state,
        manifest,
        benchmark_corpus,
        executor,
        sparse_generation,
    ))
}
