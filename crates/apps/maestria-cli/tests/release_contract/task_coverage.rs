use super::common::{TempDir, assert_index_ok, assert_init_ok, assert_ok, write_file};

fn line_value<'a>(output: &'a str, prefix: &str) -> Option<&'a str> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .map(str::trim)
}

#[test]
fn task_search_association_is_durable_without_curating_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-release-task-workspace")?;
    let instance = TempDir::new("maestria-release-task-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let workspace_path = workspace.path().to_string_lossy().into_owned();
    assert_init_ok(&instance_path, &workspace_path)?;
    write_file(
        workspace.path(),
        "notes.md",
        "A task-associated search informs the task without curating evidence.",
    )?;
    assert_index_ok(
        &instance_path,
        &workspace.path().join("notes.md").to_string_lossy(),
    )?;
    let task = assert_ok(&["task", "start", "-i", &instance_path, "Review search"])?;
    assert!(task.contains("task=1"), "{task}");

    let direct = assert_ok(&[
        "search",
        "-i",
        &instance_path,
        "--task-id",
        "1",
        "task-associated",
    ])?;
    assert!(direct.contains("rank=") || direct.contains("search_status=NoEvidenceFound"));
    let first_coverage = assert_ok(&["evidence", "coverage", "-i", &instance_path, "1"])?;
    assert!(first_coverage.contains("search_trace="), "{first_coverage}");
    assert!(
        first_coverage.contains("evidence_count=0"),
        "{first_coverage}"
    );
    let first_trace = line_value(&first_coverage, "search_trace=").ok_or("trace missing")?;

    let explained = assert_ok(&[
        "search",
        "explain",
        "-i",
        &instance_path,
        "--task-id",
        "1",
        "follow-up task-associated",
    ])?;
    assert!(explained.contains("trace_id="), "{explained}");
    let second_coverage = assert_ok(&["evidence", "coverage", "-i", &instance_path, "1"])?;
    let second_trace = line_value(&second_coverage, "search_trace=").ok_or("trace missing")?;
    assert_ne!(
        first_trace, second_trace,
        "latest associated search must win"
    );
    assert!(
        second_coverage.contains("evidence_count=0"),
        "{second_coverage}"
    );
    assert!(
        second_coverage.contains("coverage_percent="),
        "{second_coverage}"
    );
    assert!(
        second_coverage.contains("stop_reason="),
        "{second_coverage}"
    );
    Ok(())
}

#[test]
fn missing_task_id_is_rejected_before_retrieval() -> Result<(), Box<dyn std::error::Error>> {
    let instance = TempDir::new("maestria-release-task-missing")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    assert_init_ok(&instance_path, &instance_path)?;
    let (code, stdout, stderr) = super::common::run(&[
        "search",
        "-i",
        &instance_path,
        "--task-id",
        "99",
        "anything",
    ])?;
    assert_ne!(code, 0);
    assert!(stdout.is_empty());
    assert!(stderr.contains("task 99 was not found"), "{stderr}");
    Ok(())
}
