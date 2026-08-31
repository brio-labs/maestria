use maestria_cli::test_support::*;
use std::fs;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

struct DaemonHandle {
    child: std::process::Child,
}

impl Drop for DaemonHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

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

#[test]
fn retire_retrieval_events_goes_through_live_daemon_then_falls_back_locally()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::new("maestria-test-retire-daemon")?;
    let instance_root = temp.path().join("instance");
    fs::create_dir_all(&instance_root)?;
    let ip = instance_root.to_string_lossy().into_owned();
    assert_init_ok(ip.as_ref(), ip.as_ref())?;

    let _daemon = DaemonHandle {
        child: Command::new(bin()?)
            .args(["start", "-i", ip.as_ref()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    };
    // Poll until the daemon answers status with the retirement boundary
    // line instead of sleeping a fixed startup window.
    let bound = Duration::from_secs(10);
    let mut attempts = 200;
    let (_code, _status_stdout, _status_stderr) = loop {
        let attempt = match run_bounded(&["status", "-i", ip.as_ref()], bound) {
            Ok(attempt) => attempt,
            Err(_) if attempts > 0 => {
                attempts -= 1;
                thread::sleep(Duration::from_millis(25));
                continue;
            }
            other => break other?,
        };
        match &attempt {
            (0, stdout, _) if stdout.contains("retrieval_events_retired_through") => {
                break attempt;
            }
            _ if attempts > 0 => {
                attempts -= 1;
                thread::sleep(Duration::from_millis(25));
            }
            (_, stdout, stderr) => {
                return Err(format!("status never became ready: {stdout:?} {stderr:?}").into());
            }
        }
    };

    // The daemon owns the instance, so this must flow through the daemon
    // socket rather than a local mutation session.
    let (code, stdout, stderr) = run_bounded(
        &[
            "retire-retrieval-events",
            "-i",
            ip.as_ref(),
            "--before-sequence",
            "2",
            "--reason",
            "audit policy",
            "--yes",
        ],
        bound,
    )?;
    assert_eq!(
        code, 0,
        "daemon retire failed: stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stdout.contains("retired_through=2"),
        "daemon retire output missing boundary: {stdout}"
    );

    // After the daemon exits, the command falls back to a local session.
    drop(_daemon);
    let mut attempts = 200;
    loop {
        let attempt = match run_bounded(
            &[
                "retire-retrieval-events",
                "-i",
                ip.as_ref(),
                "--before-sequence",
                "3",
                "--reason",
                "after daemon",
                "--yes",
            ],
            bound,
        ) {
            Ok(attempt) => attempt,
            Err(_) if attempts > 0 => {
                attempts -= 1;
                thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(error) => return Err(error),
        };
        match &attempt {
            (0, stdout, _) if stdout.contains("retired_through=3") => break,
            _ if attempts > 0 => {
                attempts -= 1;
                thread::sleep(Duration::from_millis(25));
            }
            (_, stdout, stderr) => {
                return Err(
                    format!("local fallback never succeeded: {stdout:?} {stderr:?}").into(),
                );
            }
        }
    }
    Ok(())
}
