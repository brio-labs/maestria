use maestria_cli::test_support::*;

#[test]
fn retire_retrieval_events_records_marker_and_reports_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-retire-workspace")?;
    let instance = TempDir::new("maestria-test-retire-instance")?;
    let ip = instance.path().to_string_lossy().into_owned();
    let wp = workspace.path().to_string_lossy().into_owned();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;

    let (code, stdout, stderr) = run(&[
        "retire-retrieval-events",
        "-i",
        ip.as_ref(),
        "--before-sequence",
        "2",
        "--reason",
        "audit policy",
        "--yes",
    ])?;
    assert_eq!(
        code, 0,
        "retire failed: stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stdout.contains("retired_through=2"),
        "retire output missing boundary: {stdout}"
    );

    // A second request at or below the high-water mark is a recorded no-op.
    let (code, stdout, stderr) = run(&[
        "retire-retrieval-events",
        "-i",
        ip.as_ref(),
        "--before-sequence",
        "1",
        "--reason",
        "late lower request",
        "--yes",
    ])?;
    assert_eq!(
        code, 0,
        "repeat retire failed: stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stdout.contains("retired_through=2"),
        "high-water must not move backwards: {stdout}"
    );

    let (code, stdout, stderr) = run(&["status", "-i", ip.as_ref()])?;
    assert_eq!(
        code, 0,
        "status failed: stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stdout.contains("retrieval_events_retired_through 2"),
        "status must report the retirement boundary: {stdout}"
    );
    Ok(())
}

#[test]
fn retire_retrieval_events_rejects_empty_reason_and_zero_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-retire-reject-workspace")?;
    let instance = TempDir::new("maestria-test-retire-reject-instance")?;
    let ip = instance.path().to_string_lossy().into_owned();
    let wp = workspace.path().to_string_lossy().into_owned();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;

    let (code, _stdout, stderr) = run(&[
        "retire-retrieval-events",
        "-i",
        ip.as_ref(),
        "--before-sequence",
        "5",
        "--reason",
        "   ",
        "--yes",
    ])?;
    assert_ne!(code, 0, "empty reason must be rejected: stderr={stderr:?}");

    let (code, _stdout, stderr) = run(&[
        "retire-retrieval-events",
        "-i",
        ip.as_ref(),
        "--before-sequence",
        "0",
        "--reason",
        "zero boundary",
        "--yes",
    ])?;
    assert_ne!(code, 0, "zero boundary must be rejected: stderr={stderr:?}");
    Ok(())
}
