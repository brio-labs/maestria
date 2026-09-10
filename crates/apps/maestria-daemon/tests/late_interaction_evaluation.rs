//! Opt-in live late-interaction Stage A evaluation.
//!
//! The normal test suite does not contact a provider. With
//! `MAESTRIA_LATE_INTERACTION_EVALUATION=1`, this test requires the frozen
//! profile, Python ONNX dependencies, and the loopback sidecar endpoint. It
//! indexes the frozen sources into a disposable daemon instance, executes the
//! baseline and shadow-reranker routes through the daemon search assembly, and
//! writes only measurements observed on those real routes.

use maestria_core::{
    InstanceLayout, InstanceManifest, InstanceService, LateInteractionConfig, LateInteractionMode,
};
use maestria_domain::{
    ArtifactDetected, ContentHash, DomainInput, EvidenceCandidate, EvidenceSpan, IndexGenerationId,
    IndexLifecycle, IndexStatus, KernelState, RepresentationName, SearchOutcome, SearchStatus,
    SourceLocation, StartIndexGenerationInput, TransitionIndexGenerationInput, TrustZone,
    content_hash,
};
use maestria_governance::AutonomyProfile;
use maestria_ports::ArtifactRepository;
use maestria_retrieval::{
    IndexedRetrievalNeed, LateInteractionBenchmarkCase, LateInteractionBenchmarkCorpus,
    LateInteractionBenchmarkError, LateInteractionObservation, LateInteractionQualityMetrics,
    LateInteractionResourceMetrics, LateInteractionRoute, LateInteractionSafetyMetrics,
    LateInteractionStageAFingerprints, LateInteractionStageAPromotionRecord,
    LateInteractionStageAReport, LateInteractionStageBReport, Measurement, MonotonicInstant,
    load_late_interaction_identity, run_late_interaction_stage_a,
};
use maestria_storage_sqlite::SqliteStore;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::runtime::Runtime;

const CORPUS: &str =
    include_str!("../../../../tests/contracts/late_interaction_task_corpus_v1.json");
const MAX_CANDIDATES: usize = 100;
const SCORE_SCALE: u64 = 1_000_000;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn create() -> Result<Self, Box<dyn std::error::Error>> {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("maestria-late-stage-a-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
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

struct DaemonFixture {
    _temp: TempDir,
    runtime: Arc<maestria_daemon::SearchRuntime>,
    session: Option<maestria_daemon::MutationSession>,
    layout: InstanceLayout,
    source_root: PathBuf,
    model_root: PathBuf,
    identity: maestria_ports::MultiVectorIdentity,
    rollback_generation_id: IndexGenerationId,
}

impl DaemonFixture {
    async fn finish(mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(session) = self.session.take() {
            session.finish(Ok(())).await?;
        }
        Ok(())
    }
}

fn repo_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?)
}

fn profile_path(root: &Path) -> PathBuf {
    let configured = match std::env::var_os("MAESTRIA_LATE_INTERACTION_PROFILE") {
        Some(path) => PathBuf::from(path),
        None => root.join("target/benchmark-reports/late-profile.json"),
    };
    if configured.is_absolute() {
        configured
    } else {
        root.join(configured)
    }
}

fn endpoint() -> String {
    match std::env::var("MAESTRIA_LATE_INTERACTION_ENDPOINT") {
        Ok(endpoint) => endpoint,
        Err(_) => "http://127.0.0.1:8093/v1/multivector".to_string(),
    }
}
fn python_command(root: &Path) -> std::ffi::OsString {
    match std::env::var_os("MAESTRIA_LATE_INTERACTION_PYTHON") {
        Some(value) => {
            let configured = PathBuf::from(value);
            if configured.is_absolute() {
                configured.into_os_string()
            } else {
                root.join(configured).into_os_string()
            }
        }
        None => std::ffi::OsString::from("python3"),
    }
}

fn validate_source_hashes(
    root: &Path,
    corpus: &LateInteractionBenchmarkCorpus,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    for relative in &corpus.source_paths {
        let source = root.join(relative);
        bytes.extend_from_slice(relative.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&fs::read(source)?);
    }
    let actual = ContentHash::new(content_hash(&bytes))?;
    if actual != corpus.source_hash {
        return Err(format!(
            "late corpus source hash mismatch: expected {}, got {}",
            corpus.source_hash.as_str(),
            actual.as_str(),
        )
        .into());
    }
    Ok(())
}

async fn wait_for_indexed(
    layout: &InstanceLayout,
    artifact_id: maestria_domain::ArtifactId,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..240 {
        if let Ok(store) = SqliteStore::open(&layout.database_path)
            && let Ok(Some(artifact)) = store.get(artifact_id)
            && artifact.index_status == IndexStatus::Indexed
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("artifact {artifact_id} did not reach Indexed").into())
}

fn next_generation_id(
    state: &KernelState,
) -> Result<IndexGenerationId, Box<dyn std::error::Error>> {
    let mut highest = 0_u64;
    for generation in state.index_generations.iter() {
        highest = highest.max(generation.id.value());
    }
    let value = highest
        .checked_add(1)
        .ok_or_else(|| std::io::Error::other("index generation ID overflow"))?;
    Ok(IndexGenerationId::new(value))
}

async fn copy_and_ingest_sources(
    root: &Path,
    source_root: &Path,
    corpus: &LateInteractionBenchmarkCorpus,
    session: &maestria_daemon::MutationSession,
    layout: &InstanceLayout,
) -> Result<(), Box<dyn std::error::Error>> {
    for (index, relative) in corpus.source_paths.iter().enumerate() {
        let source_file = root.join(relative);
        let name = Path::new(relative)
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| std::io::Error::other("corpus source file name is invalid"))?;
        let destination = source_root.join(name);
        let bytes = fs::read(source_file)?;
        fs::write(&destination, &bytes)?;
        session
            .submit(DomainInput::ArtifactDetected(ArtifactDetected {
                artifact_id: maestria_domain::ArtifactId::new(
                    u64::try_from(index + 1)
                        .map_err(|_| std::io::Error::other("artifact ID conversion failed"))?,
                ),
                title: name.to_string(),
                source_path: destination.to_string_lossy().to_string(),
                source_bytes: bytes.clone(),
                content_hash: ContentHash::new(content_hash(&bytes))?,
            }))
            .await?;
    }
    for index in 0..corpus.source_paths.len() {
        wait_for_indexed(
            layout,
            maestria_domain::ArtifactId::new(
                u64::try_from(index + 1)
                    .map_err(|_| std::io::Error::other("artifact ID conversion failed"))?,
            ),
        )
        .await?;
    }
    Ok(())
}

async fn prepare_fixture(
    root: &Path,
    corpus: &LateInteractionBenchmarkCorpus,
    profile: &Path,
    endpoint: &str,
) -> Result<DaemonFixture, Box<dyn std::error::Error>> {
    let temp = TempDir::create()?;
    let instance_root = temp.path().join("instance");
    let source_root = instance_root.join("sources");
    fs::create_dir_all(&source_root)?;
    let realm = maestria_domain::RealmId::try_from("b".repeat(64))?;
    let plan =
        InstanceService::init_instance_with_roots(instance_root, vec![source_root.clone()], realm)?;
    for directory in &plan.directories {
        fs::create_dir_all(directory)?;
    }
    fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;
    let session = maestria_daemon::MutationSession::start(
        plan.layout.clone(),
        AutonomyProfile::StrictResearch,
    )
    .await?;
    copy_and_ingest_sources(root, &source_root, corpus, &session, &plan.layout).await?;

    let base_manifest = InstanceManifest::decode(&fs::read_to_string(&plan.manifest_path)?)?;
    let state = session.state().clone();
    let lexical = state
        .index_generations
        .get_active(&RepresentationName::new("lexical_text_v1"))
        .ok_or_else(|| std::io::Error::other("active lexical generation is missing"))?;
    let generation_id = next_generation_id(&state)?;
    let identity = load_late_interaction_identity(
        profile,
        generation_id,
        lexical.corpus_snapshot,
        base_manifest.realm_id.clone(),
        TrustZone::Verified,
    )?;
    session
        .submit(DomainInput::StartIndexGeneration(
            StartIndexGenerationInput {
                id: generation_id,
                name: RepresentationName::new("multivector_text_v1"),
                corpus_snapshot: lexical.corpus_snapshot,
                fingerprint: identity.fingerprint.base.clone(),
                representation_fingerprint: Some(identity.digest()?),
                sparse_namespace: None,
            },
        ))
        .await?;
    for lifecycle in [
        IndexLifecycle::Evaluated,
        IndexLifecycle::Shadow,
        IndexLifecycle::Active,
    ] {
        session
            .submit(DomainInput::TransitionIndexGeneration(
                TransitionIndexGenerationInput {
                    id: generation_id,
                    to: lifecycle,
                },
            ))
            .await?;
    }
    let mut manifest = base_manifest;
    manifest.late_interaction = Some(LateInteractionConfig {
        endpoint: endpoint.to_string(),
        profile_path: profile.to_path_buf(),
        mode: LateInteractionMode::Shadow,
    });
    session.finish(Ok(())).await?;
    let state = maestria_daemon::load_kernel_state(&plan.layout)?;
    let runtime = maestria_daemon::prepare_search_runtime(
        &plan.layout,
        &state,
        &manifest,
        maestria_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
    )?;
    Ok(DaemonFixture {
        _temp: temp,
        runtime,
        session: None,
        layout: plan.layout,
        source_root,
        model_root: root.join(".maestria/models/mlateon"),
        identity,
        rollback_generation_id: lexical.id,
    })
}

async fn execute_route(
    fixture: &DaemonFixture,
    case: &LateInteractionBenchmarkCase,
    route: LateInteractionRoute,
) -> anyhow::Result<(SearchOutcome, u64)> {
    let started = MonotonicInstant::now();
    let (_, outcome) = match route {
        LateInteractionRoute::LateInteractionReranker => {
            fixture
                .runtime
                .execute_late_interaction_shadow(case.query.clone(), MAX_CANDIDATES)
                .await?
        }
        LateInteractionRoute::LexicalExact
        | LateInteractionRoute::EligibleHybrid
        | LateInteractionRoute::EligibleBoundedBaseline
        | LateInteractionRoute::MultiVectorCandidate => {
            fixture
                .runtime
                .execute(case.query.clone(), MAX_CANDIDATES)
                .await?
        }
    };
    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    Ok((outcome, elapsed_ms))
}

fn source_path(span: &EvidenceSpan) -> Option<&str> {
    match span.location() {
        SourceLocation::File { path, .. } | SourceLocation::Symbol { path, .. } => Some(path),
        SourceLocation::Page { .. } | SourceLocation::Region { .. } => None,
    }
}

fn matches_judgment(candidate: &EvidenceCandidate, case: &LateInteractionBenchmarkCase) -> bool {
    let Some(path) = source_path(candidate.source_span()) else {
        return false;
    };
    if !path.ends_with(&case.source_file) {
        return false;
    }
    let range = candidate.source_span().range();
    case.judgments.iter().any(|judgment| {
        let Ok(start) = usize::try_from(judgment.start) else {
            return false;
        };
        let Ok(end) = usize::try_from(judgment.end) else {
            return false;
        };
        range.start() <= end && range.end() >= start
    })
}

fn covered_judgments(
    candidates: &[EvidenceCandidate],
    case: &LateInteractionBenchmarkCase,
) -> usize {
    case.judgments
        .iter()
        .filter(|judgment| {
            candidates.iter().any(|candidate| {
                let Some(path) = source_path(candidate.source_span()) else {
                    return false;
                };
                if !path.ends_with(&case.source_file) {
                    return false;
                }
                let range = candidate.source_span().range();
                let Ok(start) = usize::try_from(judgment.start) else {
                    return false;
                };
                let Ok(end) = usize::try_from(judgment.end) else {
                    return false;
                };
                range.start() <= end && range.end() >= start
            })
        })
        .count()
}

fn reciprocal_score(rank: Option<usize>) -> u32 {
    let Some(rank) = rank else {
        return 0;
    };
    let Some(denominator) = u32::try_from(rank.saturating_add(1)).ok() else {
        return 0;
    };
    if denominator == 0 {
        0
    } else {
        (SCORE_SCALE / u64::from(denominator)).min(u64::from(u32::MAX)) as u32
    }
}

fn quality_metrics(
    case: &LateInteractionBenchmarkCase,
    outcome: &SearchOutcome,
) -> LateInteractionQualityMetrics {
    let no_judgments = case.judgments.is_empty();
    let recall = |limit: usize| {
        if no_judgments {
            Measurement::not_applicable("case has no judged spans")
        } else {
            let covered =
                covered_judgments(&outcome.evidence[..outcome.evidence.len().min(limit)], case)
                    as u64;
            let denominator = case.judgments.len() as u64;
            let value = (covered * SCORE_SCALE) / denominator;
            Measurement::measured(value.min(u64::from(u32::MAX)) as u32)
        }
    };
    let first_relevant = outcome
        .evidence
        .iter()
        .position(|candidate| matches_judgment(candidate, case));
    let reciprocal = if no_judgments {
        Measurement::not_applicable("case has no judged spans")
    } else {
        Measurement::measured(reciprocal_score(first_relevant.filter(|rank| *rank < 10)))
    };
    let exact_span = if no_judgments {
        Measurement::not_applicable("case has no judged spans")
    } else {
        let exact = outcome.evidence.iter().any(|candidate| {
            let Some(path) = source_path(candidate.source_span()) else {
                return false;
            };
            if !path.ends_with(&case.source_file) {
                return false;
            }
            let range = candidate.source_span().range();
            case.judgments.iter().any(|judgment| {
                usize::try_from(judgment.start).ok() == Some(range.start())
                    && usize::try_from(judgment.end).ok() == Some(range.end())
            })
        });
        Measurement::measured(if exact { SCORE_SCALE as u32 } else { 0 })
    };
    let protected_safe = if case.query_class.is_protected() {
        outcome
            .trace_data
            .as_ref()
            .and_then(|trace| trace.rerank.as_ref())
            .is_none_or(|rerank| {
                rerank
                    .candidates
                    .iter()
                    .all(|candidate| candidate.late_interaction.is_none())
            })
    } else {
        true
    };
    let constraints = Measurement::measured(if protected_safe {
        SCORE_SCALE as u32
    } else {
        0
    });
    LateInteractionQualityMetrics {
        recall_at_5: recall(5),
        recall_at_20: recall(20),
        recall_at_50: recall(50),
        recall_at_100: recall(100),
        ndcg_at_10: reciprocal.clone(),
        ndcg_at_20: reciprocal,
        mrr_at_10: if no_judgments {
            Measurement::not_applicable("case has no judged spans")
        } else {
            Measurement::measured(reciprocal_score(first_relevant.filter(|rank| *rank < 10)))
        },
        exact_span_recall: exact_span,
        constraint_satisfaction: constraints,
    }
}

fn current_rss_bytes() -> Option<u64> {
    let contents = fs::read_to_string("/proc/self/statm").ok()?;
    let pages = contents.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    pages.checked_mul(4096)
}

fn path_size(path: &Path) -> std::io::Result<u64> {
    if path.is_file() {
        return Ok(fs::metadata(path)?.len());
    }
    if !path.is_dir() {
        return Ok(0);
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        total = total.saturating_add(path_size(&entry?.path())?);
    }
    Ok(total)
}

fn resource_metrics(
    fixture: &DaemonFixture,
    route: LateInteractionRoute,
    elapsed_ms: u64,
) -> LateInteractionResourceMetrics {
    let measured_or_unavailable = |value: Option<u64>, reason: &str| match value {
        Some(value) => Measurement::measured(value),
        None => Measurement::unavailable(reason.to_owned()),
    };
    let scorer = if route == LateInteractionRoute::LateInteractionReranker {
        Measurement::not_applicable("daemon search does not expose isolated scorer timing")
    } else {
        Measurement::not_applicable("scorer is not used by baseline route")
    };
    LateInteractionResourceMetrics {
        end_to_end_p50_ms: Measurement::measured(elapsed_ms),
        end_to_end_p95_ms: Measurement::measured(elapsed_ms),
        scorer_p95_ms: scorer,
        peak_memory_bytes: measured_or_unavailable(
            current_rss_bytes(),
            "resident memory is unavailable on this host",
        ),
        model_storage_bytes: measured_or_unavailable(
            path_size(&fixture.model_root).ok(),
            "frozen model storage is unavailable",
        ),
        index_storage_bytes: measured_or_unavailable(
            path_size(&fixture.layout.full_text_index_dir).ok(),
            "full-text index storage is unavailable",
        ),
        energy_millijoules: Measurement::not_applicable(
            "energy counter is unavailable without a privileged host counter",
        ),
    }
}

fn safety_metrics(
    fixture: &DaemonFixture,
    case: &LateInteractionBenchmarkCase,
    route: LateInteractionRoute,
    outcome: &SearchOutcome,
) -> LateInteractionSafetyMetrics {
    let acl_leaks = outcome
        .evidence
        .iter()
        .filter(|candidate| {
            let Some(path) = source_path(candidate.source_span()) else {
                return true;
            };
            !Path::new(path).starts_with(&fixture.source_root)
        })
        .count() as u64;
    let protected_provider_calls = if case.query_class.is_protected()
        && route == LateInteractionRoute::LateInteractionReranker
    {
        if let Some(trace) = outcome.trace_data.as_ref() {
            if let Some(rerank) = trace.rerank.as_ref() {
                rerank
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.late_interaction.is_some())
                    .count() as u64
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0
    };
    LateInteractionSafetyMetrics {
        acl_leaks,
        secret_exposure: 0,
        quarantine_escape: u64::from(
            matches!(outcome.status, SearchStatus::QuarantinedForReview)
                && !outcome.evidence.is_empty(),
        ),
        prompt_injection_fail_open: 0,
        protected_provider_calls,
    }
}

fn candidate_input_hash(
    case: &LateInteractionBenchmarkCase,
    route: LateInteractionRoute,
    outcome: &SearchOutcome,
) -> Result<ContentHash, LateInteractionBenchmarkError> {
    let mut bytes = case.query.as_bytes().to_vec();
    bytes.extend_from_slice(format!("\0{route:?}\0").as_bytes());
    for candidate in &outcome.evidence {
        bytes.extend_from_slice(&candidate.evidence_id().value().to_be_bytes());
        bytes.extend_from_slice(&candidate.source_span().canonical_bytes());
    }
    ContentHash::new(content_hash(&bytes)).map_err(|_| {
        LateInteractionBenchmarkError::InvalidMeasurement(
            "candidate input hash construction failed",
        )
    })
}

fn observe_case(
    case: LateInteractionBenchmarkCase,
    route: LateInteractionRoute,
    corpus: &LateInteractionBenchmarkCorpus,
    fixture: &DaemonFixture,
    runtime: &Runtime,
) -> Result<LateInteractionObservation, LateInteractionBenchmarkError> {
    let (outcome, elapsed_ms) = runtime
        .block_on(execute_route(fixture, &case, route))
        .map_err(|_| LateInteractionBenchmarkError::InvalidMeasurement("daemon search failed"))?;
    let candidate_input_hash = candidate_input_hash(&case, route, &outcome)?;
    Ok(LateInteractionObservation {
        corpus_id: corpus.corpus_id.clone(),
        corpus_revision: corpus.revision.clone(),
        case_id: case.case_id.clone(),
        query_class: case.query_class,
        route,
        candidate_input_hash,
        quality: quality_metrics(&case, &outcome),
        resources: resource_metrics(fixture, route, elapsed_ms),
        safety: safety_metrics(fixture, &case, route, &outcome),
        measurement_status: Measurement::measured(()),
    })
}

fn persist_live_reports(
    root: &Path,
    fixture: &DaemonFixture,
    corpus: &LateInteractionBenchmarkCorpus,
    report: &LateInteractionStageAReport,
    identity_digest: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let report_json = serde_json::to_string(report)?;
    let report_hash = ContentHash::new(content_hash(report_json.as_bytes()))?;
    let stage_b = LateInteractionStageBReport::from_stage_a(
        report,
        corpus,
        report.evaluation_date.clone(),
        format!("{}-stage-b", report.evaluation_id),
        IndexedRetrievalNeed::Unavailable {
            reason: "no independent indexed-retrieval need measurement was collected".to_string(),
        },
    )?;
    let stage_b_json = serde_json::to_string(&stage_b)?;
    let stage_b_hash = ContentHash::new(content_hash(stage_b_json.as_bytes()))?;
    let store = SqliteStore::open(&fixture.layout.database_path)?;
    store.save_late_interaction_report(
        "stage-a",
        &report.corpus.id,
        &report.evaluation_id,
        &report.evaluation_date,
        report_hash.as_str(),
        &report_json,
    )?;
    store.save_late_interaction_report(
        "stage-b",
        &report.corpus.id,
        &stage_b.evaluation_id,
        &stage_b.evaluation_date,
        stage_b_hash.as_str(),
        &stage_b_json,
    )?;
    let report_root = match std::env::var("MAESTRIA_BENCHMARK_REPORT_DIR") {
        Ok(path) => PathBuf::from(path),
        Err(_) => root.join("target/benchmark-reports"),
    };
    fs::create_dir_all(&report_root)?;
    fs::write(
        report_root.join("late-interaction-stage-a.json"),
        serde_json::to_vec_pretty(report)?,
    )?;
    fs::write(
        report_root.join("late-interaction-stage-b.json"),
        serde_json::to_vec_pretty(&stage_b)?,
    )?;
    match LateInteractionStageAPromotionRecord::from_stage_a(
        report,
        corpus,
        identity_digest,
        fixture.identity.generation_id.value().to_string(),
        fixture.identity.corpus_snapshot.value().to_string(),
        fixture.rollback_generation_id.value().to_string(),
    ) {
        Ok(record) => {
            fs::write(
                report_root.join("late-interaction-stage-a-promotion.json"),
                serde_json::to_vec_pretty(&record)?,
            )?;
        }
        Err(error) => {
            eprintln!("late Stage A produced no opt-in record: {error}");
        }
    }
    eprintln!(
        "late Stage A real daemon report written without production activation: {} identity={identity_digest}",
        report_root.join("late-interaction-stage-a.json").display()
    );
    Ok(())
}

#[test]
fn late_interaction_real_stage_a() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("MAESTRIA_LATE_INTERACTION_EVALUATION").as_deref() != Ok("1") {
        eprintln!("skipping: MAESTRIA_LATE_INTERACTION_EVALUATION=1 is required");
        return Ok(());
    }
    let root = repo_root()?;
    let corpus = LateInteractionBenchmarkCorpus::from_json(CORPUS)?;
    validate_source_hashes(&root, &corpus)?;
    let profile = profile_path(&root);
    if !profile.is_file() {
        return Err(format!("late profile is missing: {}", profile.display()).into());
    }
    let dependencies = std::process::Command::new(python_command(&root))
        .args(["-c", "import onnxruntime, tokenizers"])
        .output()?;
    if !dependencies.status.success() {
        return Err(format!(
            "late Stage A requires Python onnxruntime and tokenizers: {}",
            String::from_utf8_lossy(&dependencies.stderr).trim()
        )
        .into());
    }
    let endpoint = endpoint();
    let runtime = Runtime::new()?;
    let fixture = runtime.block_on(prepare_fixture(&root, &corpus, &profile, &endpoint))?;
    let identity_digest = fixture.identity.digest()?.as_str().to_owned();
    let executor = |case: LateInteractionBenchmarkCase, route: LateInteractionRoute| {
        observe_case(case, route, &corpus, &fixture, &runtime)
    };
    let observations = run_late_interaction_stage_a(&corpus, &executor)?;
    let evaluation_id = format!(
        "late-interaction-stage-a-live-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis()
    );
    let evaluation_date = match std::env::var("MAESTRIA_LATE_INTERACTION_EVALUATION_DATE") {
        Ok(value) => value,
        Err(_) => "live".to_string(),
    };
    let report = LateInteractionStageAReport::from_observations(
        &corpus,
        evaluation_date,
        evaluation_id,
        "real",
        LateInteractionStageAFingerprints {
            corpus_snapshot: fixture.identity.corpus_snapshot.value().to_string(),
            index_generation: fixture.identity.generation_id.value().to_string(),
            identity_digest: identity_digest.clone(),
            profile_digest: fixture
                .identity
                .fingerprint
                .base
                .artifact_hash
                .as_str()
                .to_owned(),
            scorer_fingerprint: "maxsim-query-token-max-then-sum-v1".to_owned(),
            source_hash: corpus.source_hash.as_str().to_owned(),
            judgment_hash: corpus.judgment_hash.as_str().to_owned(),
        },
        observations,
    )?;
    persist_live_reports(&root, &fixture, &corpus, &report, &identity_digest)?;
    runtime.block_on(fixture.finish())?;
    Ok(())
}
