mod common;

use common::{
    TempDir, assert_index_ok, assert_init_ok, assert_ok, assert_ok_lines, run, run_bounded,
    write_file,
};
use maestria_core::InstanceLayout;
use maestria_domain::{
    ArtifactId, Authority, BlobId, ClaimId, ContentHash, DomainEvent, DomainEventEnvelope, EventId,
    EvidenceId, EvidenceKind, IntegrityState, LineRange, LogicalTick, MemoryCandidateId,
    ReviewStatus, SecurityMetadata, Sensitivity, SequenceNumber, SnapshotRef, TrustZone,
};
use maestria_ports::EventLog;
use maestria_storage_sqlite::SqliteStore;
use std::path::Path;
use std::time::Duration;

fn parse_kv_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .filter_map(|token| token.split_once('='))
        .find_map(|(candidate, value)| (candidate == key).then_some(value))
}

fn assert_err(args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let (code, stdout, stderr) = run(args)?;
    assert_ne!(code, 0, "command unexpectedly succeeded: {args:?}");
    assert!(
        stdout.trim().is_empty(),
        "failed command wrote unexpected stdout: {stdout}"
    );
    Ok(stderr)
}
fn status_event_count(instance_path: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let (code, stdout, stderr) = run(&["status", "-i", instance_path])?;
    assert_eq!(code, 0, "status failed: {stderr}");
    let event_count = stdout
        .lines()
        .find_map(|line| line.strip_prefix("events "))
        .and_then(|value| value.parse().ok())
        .ok_or("status output missing event count")?;
    Ok(event_count)
}

fn search_evidence(instance_path: &str, query: &str) -> Result<String, Box<dyn std::error::Error>> {
    let stdout = assert_ok(&["search", "-i", instance_path, query])?;
    let line = stdout
        .lines()
        .find(|line| line.contains("evidence="))
        .ok_or("search output missing evidence line")?;
    parse_kv_value(line, "evidence")
        .map(str::to_owned)
        .ok_or_else(|| "search output missing evidence id".into())
}

fn index_claim(
    instance_path: &str,
    workspace: &Path,
    content: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    write_file(workspace, "notes.md", content)?;
    let file = workspace.join("notes.md").to_string_lossy().into_owned();
    assert_index_ok(instance_path, &file)?;
    let query = content
        .lines()
        .find(|line| !line.starts_with('#') && !line.trim().is_empty())
        .and_then(|line| line.split_whitespace().next())
        .ok_or("content must have a searchable word")?;
    search_evidence(instance_path, query)
}

fn seed_promotable_candidate(instance_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let layout = InstanceLayout::for_root(instance_path.to_path_buf());
    let store = SqliteStore::open(&layout.database_path)?;
    let security = SecurityMetadata {
        trust_zone: TrustZone::Verified,
        authority: Authority::User,
        integrity: IntegrityState::Verified,
        sensitivity: Sensitivity::Internal,
        review_status: ReviewStatus::Approved,
        quarantined: false,
        prompt_injection_risk: false,
        poisoning_flags: Vec::new(),
        read_allowed: true,
        write_allowed: true,
        scope_id: None,
    };
    let artifact_id = ArtifactId::new(1);
    let claim_id = ClaimId::new(1);
    let evidence_id = EvidenceId::new(1);
    let candidate_id = MemoryCandidateId::new(1);
    let events = [
        DomainEvent::ArtifactRegistered {
            artifact_id,
            title: "notes.md".to_string(),
            security: security.clone(),
        },
        DomainEvent::EvidenceRecorded {
            evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "notes.md".to_string(),
                range: LineRange::new(1, 1)?,
                snapshot: SnapshotRef::new(
                    BlobId::new(1),
                    ContentHash::new(format!("sha256:{}", "0".repeat(64)))?,
                ),
            },
            excerpt: "A trusted claim.".to_string(),
            observed_at: LogicalTick::new(1),
            security: security.clone(),
        },
        DomainEvent::ClaimCreated {
            claim_id,
            artifact_id,
            text: "A trusted claim.".to_string(),
            evidence_ids: vec![evidence_id],
            security: security.clone(),
        },
        DomainEvent::ClaimEvidenceLinked {
            claim_id,
            evidence_id,
        },
        DomainEvent::MemoryCandidateCreated {
            candidate_id,
            claim_id,
            evidence_ids: std::iter::once(evidence_id).collect(),
            confidence_milli: 750,
            security,
        },
    ];
    for (offset, event) in events.into_iter().enumerate() {
        let id = (offset + 1) as u64;
        store.append(DomainEventEnvelope {
            id: EventId::new(id),
            sequence: SequenceNumber::new(id),
            event,
        })?;
    }
    Ok(())
}

#[test]
fn memory_promote_returns_and_persists_within_bound() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-mem-promote-success-workspace")?;
    let instance = TempDir::new("maestria-test-mem-promote-success-instance")?;
    assert_init_ok(
        &instance.path().to_string_lossy(),
        &workspace.path().to_string_lossy(),
    )?;
    seed_promotable_candidate(instance.path())?;

    let instance_path = instance.path().to_string_lossy();
    let (code, stdout, stderr) = run_bounded(
        &[
            "memory",
            "promote",
            "-i",
            instance_path.as_ref(),
            "--candidate-id",
            "1",
            "--approve",
        ],
        Duration::from_secs(10),
    )?;
    assert_eq!(
        code, 0,
        "promotion failed within timeout\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("promoted candidate=1 memory=1"),
        "promotion output missing success: {stdout}"
    );

    let state = maestria_daemon::load_kernel_state(&InstanceLayout::for_root(instance.path()))?;
    assert!(
        state
            .memories
            .contains_key(&maestria_domain::MemoryId::new(1)),
        "successful promotion did not persist memory"
    );
    Ok(())
}

#[test]
fn memory_promote_respects_approval_gate() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-mem-promote-workspace")?;
    let instance = TempDir::new("maestria-test-mem-promote-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    let evidence_id = index_claim(
        ip.as_ref(),
        workspace.path(),
        "# Architecture\n\nThe project uses Rust for performance.\n",
    )?;
    let propose = assert_ok_lines(
        &[
            "memory",
            "propose",
            "--text",
            "The project uses Rust",
            "--evidence-id",
            &evidence_id,
            "--confidence-milli",
            "750",
            "-i",
            ip.as_ref(),
        ],
        1,
    )?;
    let candidate_id = parse_kv_value(&propose, "candidate")
        .ok_or("memory propose output missing candidate id")?
        .to_owned();
    let events_before = status_event_count(ip.as_ref())?;
    let promote_err = assert_err(&[
        "memory",
        "promote",
        "-i",
        ip.as_ref(),
        "--candidate-id",
        &candidate_id,
    ])?;
    assert!(
        promote_err.contains("cannot promote memory candidate")
            || promote_err.contains("approval required")
            || promote_err.contains("review"),
        "missing/unexpected promote gating error: {promote_err}"
    );
    assert_eq!(events_before, status_event_count(ip.as_ref())?);
    let security_err = assert_err(&[
        "memory",
        "promote",
        "-i",
        ip.as_ref(),
        "--candidate-id",
        &candidate_id,
        "--approve",
    ])?;
    assert!(
        security_err.contains("security metadata blocks promotion"),
        "user approval must not bypass security policy: {security_err}"
    );
    assert_eq!(events_before, status_event_count(ip.as_ref())?);
    let candidates = assert_ok_lines(&["memory", "candidates", "-i", ip.as_ref()], 1)?;
    assert!(candidates.contains(&format!("candidate={candidate_id}")));
    Ok(())
}

#[test]
fn reindex_hides_superseded_artifact_versions_from_default_search()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-reindex-workspace")?;
    let instance = TempDir::new("maestria-test-reindex-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    let file = workspace.path().join("notes.md");
    write_file(
        workspace.path(),
        "notes.md",
        "# Version one\n\nThe superseded alpha text.\n",
    )?;
    assert_index_ok(ip.as_ref(), &file.to_string_lossy())?;
    let first_evidence = search_evidence(ip.as_ref(), "superseded alpha")?;
    write_file(
        workspace.path(),
        "notes.md",
        "# Version two\n\nThe current beta text.\n",
    )?;
    assert_index_ok(ip.as_ref(), &file.to_string_lossy())?;
    assert!(assert_ok(&["search", "-i", ip.as_ref(), "current beta"])?.contains("evidence="));
    let superseded = assert_ok(&["search", "-i", ip.as_ref(), "superseded alpha"])?;
    assert!(!superseded.contains(&format!("evidence={first_evidence}")));
    Ok(())
}
