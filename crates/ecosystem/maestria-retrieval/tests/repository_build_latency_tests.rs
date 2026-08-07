//! Repository build-latency benchmark (Rule 44).
//!
//! Generates a fixture Cargo workspace with N symbol-bearing Rust files,
//! times cold full builds of the real extraction pipeline
//! ([`RepositoryCodeIndex::build`]), and records p50/p95 build latency per
//! corpus size to `target/benchmark-reports/repository-build-latency.json`
//! so latency regressions block promotion from the evidence ledger.
//!
//! Fixtures live in the OS temp directory. The `cargo metadata` subprocess
//! spawned by the build resolves the toolchain through the inherited
//! `RUSTUP_TOOLCHAIN` variable that the rustup cargo proxy sets for `cargo
//! test`, so no ambient default toolchain is required. The fixture root is a
//! fresh git commit, mirroring how real repository builds are measured.

use maestria_code_intel::RepositoryCodeIndex;
use maestria_retrieval::MonotonicInstant;
use serde::Serialize;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Number of symbol-bearing files in each generated fixture workspace.
const BUILD_LATENCY_SIZES: &[usize] = &[50, 200];
/// Full builds timed per corpus size; percentiles are computed over them.
const BUILD_LATENCY_RUNS: usize = 5;
/// Frozen corpus whose repository resource evidence this benchmark feeds.
const CORPUS_ID: &str = "rust-repository-frozen-v1";
/// Model fingerprint of the deterministic repository code index.
const MODEL_FINGERPRINT: &str = "repository-code-index-v3";

#[derive(Serialize)]
struct BuildLatencyReport {
    measurement_kind: &'static str,
    evaluation_date: String,
    corpus_id: &'static str,
    repository_revision: String,
    index_generation: String,
    model_fingerprint: &'static str,
    sizes: Vec<BuildLatencySize>,
}

#[derive(Serialize)]
struct BuildLatencySize {
    files: usize,
    symbols: usize,
    runs: usize,
    p50_ms: u128,
    p95_ms: u128,
    measurements_ms: Vec<u128>,
}

#[test]
fn repository_build_latency_is_reported_for_frozen_corpus_sizes()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let report_dir = workspace_root.join("target/benchmark-reports");
    fs::create_dir_all(&report_dir)?;

    let mut sizes = Vec::new();
    for &files in BUILD_LATENCY_SIZES {
        let fixture = tempfile::tempdir()?;
        write_fixture_workspace(fixture.path(), files)?;
        let mut measurements = Vec::with_capacity(BUILD_LATENCY_RUNS);
        let mut symbol_count = 0_usize;
        for run in 0..BUILD_LATENCY_RUNS {
            let started = MonotonicInstant::now();
            let index = RepositoryCodeIndex::build(
                fixture.path(),
                maestria_code_intel::REPOSITORY_CODE_PARSER_GENERATION,
            )?;
            measurements.push(started.elapsed().as_millis());
            // The fixture crate root (`lib.rs`) carries one module symbol, so
            // every generated module plus the root must be indexed.
            assert_eq!(
                index.summary.file_count,
                files + 1,
                "every generated module must carry indexed symbols"
            );
            if run == 0 {
                symbol_count = index.summary.symbol_count;
            } else {
                assert_eq!(
                    index.summary.symbol_count, symbol_count,
                    "extraction must be deterministic across runs"
                );
            }
        }
        let mut sorted = measurements.clone();
        sorted.sort_unstable();
        sizes.push(BuildLatencySize {
            files,
            symbols: symbol_count,
            runs: BUILD_LATENCY_RUNS,
            p50_ms: percentile(&sorted, 50),
            p95_ms: percentile(&sorted, 95),
            measurements_ms: measurements,
        });
    }

    let report = BuildLatencyReport {
        measurement_kind: "repository_build_latency",
        evaluation_date: evaluation_date(),
        corpus_id: CORPUS_ID,
        // Revision of the checkout under test; the generated fixtures are
        // synthetic, so their identity is the code that produced them.
        repository_revision: git_revision(&workspace_root)?,
        index_generation: maestria_code_intel::REPOSITORY_CODE_PARSER_GENERATION.to_string(),
        model_fingerprint: MODEL_FINGERPRINT,
        sizes,
    };
    let report_path = report_dir.join("repository-build-latency.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    assert!(
        report_path.is_file(),
        "build-latency report must be written for the evidence ledger"
    );
    Ok(())
}

/// Nearest-rank percentile over ascending measurements (deterministic).
///
/// `sorted` must be non-empty; percentiles clamp to the largest sample.
fn percentile(sorted: &[u128], percent: u64) -> u128 {
    let rank = (sorted.len() as u64 * percent).div_ceil(100);
    sorted[(rank.max(1) - 1) as usize]
}

/// Write a Cargo workspace with `file_count` symbol-bearing Rust modules and
/// commit it to a fresh git repository so identity discovery sees a clean
/// worktree, matching how real repository builds are measured.
fn write_fixture_workspace(root: &Path, file_count: usize) -> Result<(), Box<dyn Error>> {
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"fixture_crate\"]\nresolver = \"2\"\n",
    )?;
    let crate_root = root.join("fixture_crate");
    let src = crate_root.join("src");
    fs::create_dir_all(&src)?;
    fs::write(
        crate_root.join("Cargo.toml"),
        "[package]\nname = \"fixture_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )?;
    let mut lib = String::from("//! Generated benchmark fixture crate.\n");
    for index in 0..file_count {
        let module = format!("file_{index:04}");
        let source = format!(
            "//! Symbol-bearing module {index}.\n\n\
             pub struct Widget{index:04} {{\n    pub value: i32,\n}}\n\n\
             impl Widget{index:04} {{\n    pub fn describe(&self) -> i32 {{ self.value }}\n}}\n\n\
             pub fn helper_{index:04}(input: i32) -> i32 {{ input + 1 }}\n"
        );
        fs::write(src.join(format!("{module}.rs")), source)?;
        lib.push_str(&format!("pub mod {module};\n"));
    }
    fs::write(src.join("lib.rs"), lib)?;
    run_git(root, &["init", "--initial-branch", "main"], "git init")?;
    run_git(
        root,
        &["config", "user.email", "benchmark@example.com"],
        "git config user.email",
    )?;
    run_git(
        root,
        &["config", "user.name", "Maestria Benchmark"],
        "git config user.name",
    )?;
    run_git(root, &["add", "."], "git add")?;
    run_git(root, &["commit", "-m", "fixture workspace"], "git commit")?;
    Ok(())
}

fn run_git(root: &Path, args: &[&str], operation: &str) -> Result<(), Box<dyn Error>> {
    let status = Command::new("git").current_dir(root).args(args).status()?;
    if !status.success() {
        return Err(format!("{operation} failed: exit {status}").into());
    }
    Ok(())
}

/// HEAD revision of the checkout that generated the measurements.
fn git_revision(root: &Path) -> Result<String, Box<dyn Error>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()?;
    if !output.status.success() {
        return Err("git rev-parse HEAD failed".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Unix epoch seconds, matching the repository benchmark reports' convention.
fn evaluation_date() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "unknown".to_string(),
    }
}
