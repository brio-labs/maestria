mod common;

use std::{
    fs,
    process::{self, Command, Stdio},
    thread,
    time::Duration,
};

use common::*;

struct DaemonHandle {
    child: process::Child,
}

impl Drop for DaemonHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn daemon_instance_serves_search_status_and_open_evidence_while_running()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = common::TempDir::new("maestria-cli-daemon-query")?;
    let instance_root = temp.path().join("instance");
    let notes_dir = instance_root.join("notes");
    fs::create_dir_all(&notes_dir)?;

    let instance_path = instance_root.to_string_lossy();
    write_file(
        &notes_dir,
        "knowledge.txt",
        "# Query Lock Test\nAn indexed note that should be searchable.\n",
    )?;

    assert_init_ok(&instance_path, &instance_path)?;
    assert_index_ok(
        &instance_path,
        &notes_dir.join("knowledge.txt").to_string_lossy(),
    )?;

    let (_search_code, search_stdout, _search_stderr) =
        run(&["search", "-i", &instance_path, "lock test"])?;
    let evidence_id = extract_evidence_id(&search_stdout)?;

    let _daemon = DaemonHandle {
        child: Command::new(bin()?)
            .args(["start", "-i", instance_path.as_ref()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    };
    thread::sleep(Duration::from_millis(200));

    let query_timeout = Duration::from_secs(2);

    let (_status_code, status_stdout, _status_stderr) =
        run_bounded(&["status", "-i", &instance_path], query_timeout)?;
    assert!(status_stdout.contains("events "));

    let (search_live_code, search_live_stdout, search_live_stderr) = run_bounded(
        &["search", "-i", &instance_path, "lock test"],
        query_timeout,
    )?;
    assert!(
        search_live_stdout.contains("evidence="),
        "live search failed code={search_live_code} stdout={search_live_stdout} stderr={search_live_stderr}"
    );

    let (evidence_code, evidence_stdout, evidence_stderr) = run_bounded(
        &[
            "open-evidence",
            "-i",
            &instance_path,
            "--evidence-id",
            evidence_id.as_str(),
        ],
        query_timeout,
    )?;
    assert!(
        evidence_stdout.contains("excerpt="),
        "open evidence failed code={evidence_code} stdout={evidence_stdout} stderr={evidence_stderr}"
    );

    Ok(())
}

fn extract_evidence_id(output: &str) -> Result<String, Box<dyn std::error::Error>> {
    let evidence_line = output
        .lines()
        .find(|line| line.contains("evidence="))
        .ok_or("search output did not include an evidence candidate")?;
    let evidence_token = evidence_line
        .split_whitespace()
        .find(|token| token.starts_with("evidence="))
        .ok_or("evidence output token missing")?;
    let evidence_id = evidence_token
        .split_once('=')
        .ok_or("evidence token malformed")?
        .1;
    if evidence_id.parse::<u64>().is_err() {
        return Err(format!("invalid evidence id from search output: {evidence_token}").into());
    }
    Ok(evidence_id.to_string())
}
