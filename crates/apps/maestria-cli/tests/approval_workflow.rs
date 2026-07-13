mod common;
use common::*;

fn extract_approval_id(list_output: &str) -> String {
    let mut result = String::new();
    for line in list_output.lines() {
        if line.contains("task_activation") {
            let mut words = line.split_whitespace();
            words.next();
            if let Some(id) = words.next() {
                result = id.to_string();
                break;
            }
        }
    }
    result
}
#[test]
fn approval_list_shows_pending_high_priority_task() {
    let workspace = TempDir::new("maestria-test-workspace");
    let instance = TempDir::new("maestria-test-instance");
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref());
    write_file(workspace.path(), "notes.md", "# Notes\n\nConsensus.\n");
    let notes = workspace
        .path()
        .join("notes.md")
        .to_string_lossy()
        .into_owned();
    assert_index_ok(ip.as_ref(), &notes);
    let stdout = assert_ok(&[
        "task",
        "start",
        "-i",
        ip.as_ref(),
        "--priority",
        "high",
        "Review",
    ]);
    assert!(
        stdout.contains("task="),
        "task start output missing task id"
    );
    let list = assert_ok(&["approval", "list", "-i", ip.as_ref()]);
    assert!(
        list.contains("task_activation"),
        "approval list missing effect kind"
    );
    assert!(
        list.contains("Pending"),
        "approval status should be Pending"
    );
    assert!(list.contains("Medium"), "risk should be Medium");
}

#[test]
fn approval_approve_activates_task() {
    let workspace = TempDir::new("maestria-test-workspace");
    let instance = TempDir::new("maestria-test-instance");
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref());
    write_file(workspace.path(), "notes.md", "# Notes\n\nConsensus.\n");
    let notes = workspace
        .path()
        .join("notes.md")
        .to_string_lossy()
        .into_owned();
    assert_index_ok(ip.as_ref(), &notes);
    let task_out = assert_ok(&[
        "task",
        "start",
        "-i",
        ip.as_ref(),
        "--priority",
        "high",
        "Review",
    ]);
    let task_id: String = task_out
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    assert!(!task_id.is_empty(), "must extract task ID");
    let list = assert_ok(&["approval", "list", "-i", ip.as_ref()]);
    let approval_id = extract_approval_id(&list);
    assert!(
        !approval_id.is_empty(),
        "must extract approval ID from list"
    );
    let resolve = assert_ok(&[
        "approval",
        "resolve",
        "-i",
        ip.as_ref(),
        &approval_id,
        "--approve",
    ]);
    assert!(
        resolve.contains("Approved"),
        "resolve should confirm approval"
    );
    let show = assert_ok(&["task", "show", "-i", ip.as_ref(), &task_id]);
    assert!(
        show.contains("Active"),
        "task should be Active after approval"
    );
    let list2 = assert_ok(&["approval", "list", "-i", ip.as_ref()]);
    assert!(
        !list2.contains("Pending"),
        "no pending requests after resolution"
    );
}

#[test]
fn approval_deny_blocks_task() {
    let workspace = TempDir::new("maestria-test-workspace");
    let instance = TempDir::new("maestria-test-instance");
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref());
    write_file(workspace.path(), "notes.md", "# Notes\n");
    let notes = workspace
        .path()
        .join("notes.md")
        .to_string_lossy()
        .into_owned();
    assert_index_ok(ip.as_ref(), &notes);
    let task_out = assert_ok(&[
        "task",
        "start",
        "-i",
        ip.as_ref(),
        "--priority",
        "high",
        "Review",
    ]);
    let task_id: String = task_out
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let list = assert_ok(&["approval", "list", "-i", ip.as_ref()]);
    let approval_id = extract_approval_id(&list);
    let resolve = assert_ok(&[
        "approval",
        "resolve",
        "-i",
        ip.as_ref(),
        &approval_id,
        "--deny",
    ]);
    assert!(resolve.contains("Denied"), "resolve should confirm denial");
    let show = assert_ok(&["task", "show", "-i", ip.as_ref(), &task_id]);
    assert!(
        show.contains("Draft"),
        "task should remain Draft after denial from Draft: {show}"
    );
}

#[test]
fn approval_resolve_missing_id_errors() {
    let instance = TempDir::new("maestria-test-instance");
    let ip = instance.path().to_string_lossy();
    let wp = "/tmp";
    assert_init_ok(ip.as_ref(), wp);
    let (code, stdout, stderr) =
        run(&["approval", "resolve", "-i", ip.as_ref(), "999", "--approve"]);
    assert_ne!(code, 0, "resolve missing ID should fail");
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("not found"),
        "error should mention not found: {combined}"
    );
}

#[test]
fn approval_resolve_duplicate_is_rejected() {
    let workspace = TempDir::new("maestria-test-workspace");
    let instance = TempDir::new("maestria-test-instance");
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref());
    write_file(workspace.path(), "notes.md", "# Notes\n");
    let notes = workspace
        .path()
        .join("notes.md")
        .to_string_lossy()
        .into_owned();
    assert_index_ok(ip.as_ref(), &notes);
    assert_ok(&[
        "task",
        "start",
        "-i",
        ip.as_ref(),
        "--priority",
        "high",
        "Review",
    ]);
    let list = assert_ok(&["approval", "list", "-i", ip.as_ref()]);
    let approval_id = extract_approval_id(&list);
    assert_ok(&[
        "approval",
        "resolve",
        "-i",
        ip.as_ref(),
        &approval_id,
        "--approve",
    ]);
    let (code, stdout, stderr) = run(&[
        "approval",
        "resolve",
        "-i",
        ip.as_ref(),
        &approval_id,
        "--deny",
    ]);
    assert_ne!(code, 0, "duplicate resolve should fail");
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("already resolved"),
        "error should mention already resolved: {combined}"
    );
}
