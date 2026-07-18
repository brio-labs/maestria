mod common;

use std::{
    fs,
    path::PathBuf,
    process::{self, Command, Stdio},
    thread,
    time::Duration,
};

use common::*;
use maestria_core::InstanceLayout;
use maestria_daemon::ClientRequest;
use maestria_daemon::{ClientOperation, ClientResponse, DaemonClient};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type TextResult = Result<String, Box<dyn std::error::Error>>;

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
fn daemon_api_serves_authenticated_search_evidence_task_and_status() -> TestResult {
    let temp = TempDir::new("maestria-daemon-api")?;
    let instance_root = temp.path().join("instance");
    let notes_dir = instance_root.join("notes");
    fs::create_dir_all(&notes_dir)?;
    let instance_path = instance_root.to_string_lossy().into_owned();
    write_file(
        &notes_dir,
        "knowledge.txt",
        "# API Boundary\nThe daemon exposes source-grounded evidence.\n",
    )?;

    assert_init_ok(&instance_path, &instance_path)?;
    assert_index_ok(
        &instance_path,
        &notes_dir.join("knowledge.txt").to_string_lossy(),
    )?;
    let task_output = assert_ok(&["task", "start", "-i", &instance_path, "API task"])?;
    let task_id = match parse_kv_value(&task_output, "task") {
        Some(task_id) => task_id,
        None => return Err("task id missing".into()),
    };
    let search_output = assert_ok(&["search", "-i", &instance_path, "source grounded"])?;
    let evidence_id = extract_evidence_id(&search_output)?;

    let _daemon = DaemonHandle {
        child: Command::new(daemon_bin()?)
            .args(["-i", &instance_path])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    };
    let layout = InstanceLayout::for_root(instance_root);
    wait_for_api_files(&layout)?;
    let unauthorized = runtime_auth_request(&layout)?;
    assert!(unauthorized.contains("unauthorized"));
    let client = DaemonClient::from_instance(&layout)?;
    let runtime = tokio::runtime::Runtime::new()?;

    let status = runtime.block_on(client.request(ClientOperation::Status))?;
    let ClientResponse::Status(status) = status else {
        return Err("status request returned the wrong response type".into());
    };
    assert_eq!(status.task_count, 1);
    assert!(status.event_count > 0);

    let search = runtime.block_on(client.request(ClientOperation::Search {
        query: "source grounded".to_string(),
        limit: 5,
    }))?;
    let ClientResponse::Search(search) = search else {
        return Err("search request returned the wrong response type".into());
    };
    let candidate = match search
        .evidence
        .iter()
        .find(|candidate| candidate.evidence_id == evidence_id)
    {
        Some(candidate) => candidate,
        None => return Err("API search did not return the indexed evidence".into()),
    };

    let evidence = runtime.block_on(client.request(ClientOperation::Evidence {
        evidence_id: candidate.evidence_id,
    }))?;
    let ClientResponse::Evidence(evidence) = evidence else {
        return Err("evidence request returned the wrong response type".into());
    };
    assert!(evidence.excerpt.contains("source-grounded"));

    let tasks = runtime.block_on(client.request(ClientOperation::Task {
        task_id: Some(task_id),
    }))?;
    let ClientResponse::Task(tasks) = tasks else {
        return Err("task request returned the wrong response type".into());
    };
    assert_eq!(tasks.tasks.len(), 1);
    assert_eq!(tasks.tasks[0].title, "API task");
    Ok(())
}

fn runtime_auth_request(layout: &InstanceLayout) -> TextResult {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let mut stream = UnixStream::connect(layout.system_dir.join("daemon.sock")).await?;
        let request = ClientRequest {
            token: "0".repeat(64),
            operation: ClientOperation::Status,
        };
        let mut line = serde_json::to_vec(&request)?;
        line.push(b'\n');
        stream.write_all(&line).await?;
        let mut response = Vec::new();
        BufReader::new(stream)
            .read_until(b'\n', &mut response)
            .await?;
        Ok(String::from_utf8(response)?)
    })
}

fn daemon_bin() -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_maestria-daemon") {
        return Ok(path);
    }
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let target_dir = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(path) => PathBuf::from(path),
        None => workspace_root.join("target"),
    };
    let binary = target_dir.join("debug").join("maestria-daemon");
    if binary.is_file() {
        return Ok(binary.display().to_string());
    }
    Err(format!("daemon executable is unavailable at {}", binary.display()).into())
}

fn wait_for_api_files(layout: &InstanceLayout) -> TestResult {
    let socket = layout.system_dir.join("daemon.sock");
    let token = layout.system_dir.join("daemon.token");
    for _ in 0..200 {
        if socket.exists() && token.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(format!(
        "daemon API did not create socket and token: {} {}",
        socket.display(),
        token.display()
    )
    .into())
}

fn parse_kv_value(output: &str, key: &str) -> Option<u64> {
    for token in output.split_whitespace() {
        if let Some(value) = token.strip_prefix(&format!("{key}="))
            && let Ok(parsed) = value.parse::<u64>()
        {
            return Some(parsed);
        }
    }
    None
}

fn extract_evidence_id(output: &str) -> Result<u64, Box<dyn std::error::Error>> {
    for line in output.lines() {
        if !line.contains("evidence=") {
            continue;
        }
        for token in line.split_whitespace() {
            if let Some(value) = token.strip_prefix("evidence=") {
                return value
                    .parse::<u64>()
                    .map_err(|error| format!("invalid evidence id: {error}").into());
            }
        }
    }
    Err("search output did not include an evidence candidate".into())
}
