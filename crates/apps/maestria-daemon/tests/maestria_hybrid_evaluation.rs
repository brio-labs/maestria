//! Real hybrid (lexical + dense) evaluation for a live maestria instance.
//!
//! Manual, never CI. Requires `MAESTRIA_HYBRID_EVALUATION=1`, a stopped
//! daemon (the evaluation takes the instance write lock and runs the real
//! lifecycle operations on the projection), and readable RAPL energy
//! counters (`/sys/class/powercap/intel-rapl:0/energy_uj`). The corpus is
//! the frozen maestria-repo judgment set (`tests/contracts/`); the
//! evaluation measures the lexical route against the production hybrid
//! route (lexical + dense fusion) and saves the hybrid promotion record
//! when the dense lane wins an eligible class with complete telemetry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{ContentHash, content_hash};
use maestria_retrieval::{
    LearnedSparseBenchmarkComparison, LearnedSparseBenchmarkCorpus, LearnedSparseQueryClass,
};
use maestria_storage_sqlite::SqliteStore;

use maestria_daemon::LearnedSparseBenchmarkExecutor;

const CORPUS: &str = include_str!("../../../../tests/contracts/maestria_hybrid_corpus_v1.json");
const EVALUATION_ID: &str = "maestria-dense-hybrid-2026-08-15";
const REPORT_DIR_REL: &str = "system/benchmark-reports";

fn repo_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    Ok(root.canonicalize()?)
}

fn corpus() -> Result<LearnedSparseBenchmarkCorpus, Box<dyn std::error::Error>> {
    let corpus = LearnedSparseBenchmarkCorpus::from_json(CORPUS)?;
    Ok(corpus)
}

/// Maps the corpus source paths (repository-relative) to their source ids
/// for candidate-span matching; the retrieved candidates carry absolute
/// paths.
fn source_ids(root: &Path) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
    let corpus = corpus()?;
    let mut ids = BTreeMap::new();
    for case in corpus.cases {
        let Some(maestria_retrieval::LearnedSparseExpectedOutcome::Evidence {
            accepted_spans, ..
        }) = case.expected.as_ref()
        else {
            continue;
        };
        for span in accepted_spans {
            ids.insert(
                root.join(&span.source_id).to_string_lossy().into_owned(),
                span.source_id.clone(),
            );
        }
    }
    Ok(ids)
}

/// Persists the hybrid promotion record when the dense lane wins for at
/// least one eligible class; the record is bound to the report hash.
fn record_hybrid_decision(
    layout: &InstanceLayout,
    corpus: &LearnedSparseBenchmarkCorpus,
    comparison: &LearnedSparseBenchmarkComparison,
    report_hash: &ContentHash,
) -> Result<(), Box<dyn std::error::Error>> {
    let winners = comparison.hybrid_winning_classes();
    println!("== hybrid (dense lane) winning classes ==");
    for class in &winners {
        println!("{class:?}");
    }
    if winners.is_empty() {
        return Ok(());
    }
    let served_classes = winners.into_iter().collect();
    let record = maestria_retrieval::HybridPromotionRecord::new(
        EVALUATION_ID.to_string(),
        corpus.evaluation_date.clone(),
        served_classes,
    )
    .ok_or("hybrid promotion record construction failed")?;
    let store = SqliteStore::open(&layout.database_path)
        .map_err(|error| format!("open sqlite store: {error}"))?;
    store
        .save_hybrid_promotion_record(
            &corpus.corpus_id,
            EVALUATION_ID,
            &corpus.evaluation_date,
            report_hash.as_str(),
            &serde_json::to_string(&record).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("save hybrid promotion record: {error}"))?;
    println!("== hybrid promotion record saved to the instance store ==");
    Ok(())
}

/// Writes the dated evaluation report (serialized observations plus the
/// per-class comparison) into the instance, and returns its path.
fn emit_report(
    layout: &InstanceLayout,
    corpus: &LearnedSparseBenchmarkCorpus,
    observations: &[maestria_retrieval::LearnedSparseBenchmarkObservation],
    comparison: &LearnedSparseBenchmarkComparison,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[derive(serde::Serialize)]
    struct Report<'a> {
        evaluation_id: &'static str,
        evaluation_date: &'a str,
        corpus_id: &'a str,
        corpus_revision: &'a str,
        judgment_set_id: &'a str,
        source_input_hash: &'a str,
        environment: &'a maestria_retrieval::LearnedSparseEnvironment,
        observations: &'a [maestria_retrieval::LearnedSparseBenchmarkObservation],
        classes:
            &'a BTreeMap<LearnedSparseQueryClass, maestria_retrieval::LearnedSparseClassComparison>,
        hybrid_winning_classes: Vec<LearnedSparseQueryClass>,
    }
    let report = Report {
        evaluation_id: EVALUATION_ID,
        evaluation_date: &corpus.evaluation_date,
        corpus_id: &corpus.corpus_id,
        corpus_revision: &corpus.corpus_revision,
        judgment_set_id: &corpus.judgment_set_id,
        source_input_hash: &corpus.source_input_hash,
        environment: &corpus.environment,
        observations,
        classes: comparison.classes(),
        hybrid_winning_classes: comparison.hybrid_winning_classes(),
    };
    let report_dir = layout.root.join(REPORT_DIR_REL);
    std::fs::create_dir_all(&report_dir).map_err(|error| format!("create report dir: {error}"))?;
    let report_path = report_dir.join(format!("maestria-dense-{}.json", corpus.evaluation_date));
    std::fs::write(&report_path, serde_json::to_string_pretty(&report)?)
        .map_err(|error| format!("write report: {error}"))?;
    Ok(report_path)
}

#[tokio::test]
#[ignore = "requires a live instance, readable RAPL energy, and a stopped daemon"]
async fn maestria_hybrid_real_evaluation() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("MAESTRIA_HYBRID_EVALUATION").as_deref() != Ok("1") {
        eprintln!("skipping: MAESTRIA_HYBRID_EVALUATION=1 is required");
        return Ok(());
    }
    let instance_dir = std::env::var("MAESTRIA_EVALUATION_INSTANCE").map_err(|_| {
        "MAESTRIA_EVALUATION_INSTANCE must name the live instance directory".to_string()
    })?;
    let root = repo_root()?;
    let layout = InstanceLayout::for_root(PathBuf::from(&instance_dir));
    let manifest = InstanceManifest::decode(&std::fs::read_to_string(&layout.manifest_path)?)?;
    let mut state = maestria_daemon::load_kernel_state(&layout)?;
    let benchmark_corpus = corpus()?;
    let ids = source_ids(&root)?;
    let chunks = state.chunks.values().cloned().collect::<Vec<_>>();
    println!(
        "evaluating instance={instance_dir} corpus={} cases={} chunks={} routes={:?}",
        benchmark_corpus.corpus_id,
        benchmark_corpus.cases.len(),
        chunks.len(),
        benchmark_corpus
            .route_configurations
            .keys()
            .collect::<Vec<_>>()
    );

    let executor = LearnedSparseBenchmarkExecutor::prepare(
        &layout,
        &mut state,
        &manifest,
        benchmark_corpus.clone(),
        ids,
        chunks,
    )?;
    let observations =
        maestria_retrieval::run_learned_sparse_benchmark(&benchmark_corpus, &executor)?;
    let comparison = LearnedSparseBenchmarkComparison::evaluate(&benchmark_corpus, &observations)?;

    println!("== per-class route metrics ==");
    for (class, entry) in comparison.classes() {
        for (route, metrics) in &entry.routes {
            let recall_5 = metrics.quality.recall_at_5.measured_value();
            let recall_20 = metrics.quality.recall_at_20.measured_value();
            let p50 = metrics.resources.p50_latency_ms.measured_value();
            let p95 = metrics.resources.p95_latency_ms.measured_value();
            let energy = metrics.safety.energy.measured_value();
            println!(
                "{class:?} {route:?}: recall@5={recall_5:?} recall@20={recall_20:?} \
                 p50={p50:?}ms p95={p95:?}ms energy={energy:?}mJ violations={}",
                metrics.budget_violations
            );
        }
    }

    let report_path = emit_report(&layout, &benchmark_corpus, &observations, &comparison)?;
    let report_hash = ContentHash::new(content_hash(&std::fs::read(&report_path)?))?;
    println!(
        "report written: {} (sha256 {})",
        report_path.display(),
        report_hash.as_str()
    );
    record_hybrid_decision(&layout, &benchmark_corpus, &comparison, &report_hash)?;
    Ok(())
}
