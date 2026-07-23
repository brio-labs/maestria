use super::common::{TempDir, assert_index_ok, assert_init_ok, assert_ok, write_file};

fn line_value<'a>(output: &'a str, prefix: &str) -> Option<&'a str> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .map(str::trim)
}

fn status_event_count(instance_path: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let status = assert_ok(&["status", "-i", instance_path])?;
    Ok(
        match status
            .lines()
            .find_map(|line| line.strip_prefix("events "))
            .and_then(|value| value.parse().ok())
        {
            Some(value) => value,
            None => {
                let _ = ();
                0
            }
        },
    )
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

    let events_before_direct = status_event_count(&instance_path)?;
    let direct = assert_ok(&[
        "search",
        "-i",
        &instance_path,
        "--task-id",
        "1",
        "task-associated",
    ])?;
    assert!(direct.contains("rank=") || direct.contains("search_status=NoEvidenceFound"));
    assert_eq!(
        status_event_count(&instance_path)?,
        events_before_direct + 1,
        "direct search must append exactly one completion event",
    );
    let first_coverage = assert_ok(&["evidence", "coverage", "-i", &instance_path, "1"])?;
    assert!(first_coverage.contains("search_trace="), "{first_coverage}");
    assert!(
        first_coverage.contains("evidence_count=0"),
        "{first_coverage}"
    );
    let first_trace = line_value(&first_coverage, "search_trace=").ok_or("trace missing")?;

    let events_before_explain = status_event_count(&instance_path)?;
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
    assert_eq!(
        status_event_count(&instance_path)?,
        events_before_explain + 1,
        "explain search must append exactly one completion event",
    );
    let second_coverage = assert_ok(&["evidence", "coverage", "-i", &instance_path, "1"])?;
    let second_trace = line_value(&second_coverage, "search_trace=").ok_or("trace missing")?;
    assert_ne!(
        first_trace, second_trace,
        "latest associated search must win"
    );
    let explained_trace = line_value(&explained, "trace_id=").ok_or("trace missing")?;
    assert_eq!(
        explained_trace, second_trace,
        "coverage must expose the explain search trace"
    );
    let coverage_status = line_value(&second_coverage, "search_status=").ok_or("status missing")?;
    let explained_status = line_value(&explained, "status=").ok_or("status missing")?;
    assert_eq!(coverage_status, explained_status);
    let coverage_conflicts =
        line_value(&second_coverage, "conflicts=").ok_or("conflicts missing")?;
    let explained_conflicts = line_value(&explained, "conflicts=").ok_or("conflicts missing")?;
    assert_eq!(coverage_conflicts, explained_conflicts);
    let coverage_stop = line_value(&second_coverage, "stop_reason=").ok_or("stop missing")?;
    let explained_stop = line_value(&explained, "stop_reason=").ok_or("stop missing")?;
    assert_eq!(coverage_stop, explained_stop);
    assert!(explained.contains("retrievers_run="), "{explained}");
    assert!(explained.contains("raw_candidates="), "{explained}");
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
