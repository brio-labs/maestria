use std::fs;

use maestria_cli::test_support::{
    TempDir, assert_err, assert_index_ok, assert_init_ok, assert_task_start, write_file,
};

#[test]
fn query_commands_require_an_initialized_instance() -> Result<(), Box<dyn std::error::Error>> {
    let instance = TempDir::new("maestria-test-uninitialized-instance")?;
    let error = assert_err(&[
        "search",
        "-i",
        &instance.path().to_string_lossy(),
        "anything",
    ])?;
    assert!(
        error.contains("instance manifest is missing"),
        "unexpected uninitialized-instance error: {error}"
    );
    let error = assert_err(&[
        "open-evidence",
        "-i",
        &instance.path().to_string_lossy(),
        "--evidence-id",
        "1",
    ])?;
    assert!(
        error.contains("instance manifest is missing"),
        "unexpected uninitialized-instance evidence error: {error}"
    );
    Ok(())
}

#[test]
fn read_commands_reject_existing_database_without_valid_manifest()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-read-manifest-workspace")?;
    let instance = TempDir::new("maestria-test-read-manifest-instance")?;
    let ip = instance.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), workspace.path().to_string_lossy().as_ref())?;
    let _task_id = assert_task_start(ip.as_ref(), "Persisted before manifest removal")?;
    write_file(
        workspace.path(),
        "read-validation.md",
        "persisted state must still require a valid manifest",
    )?;
    assert_index_ok(
        ip.as_ref(),
        &workspace
            .path()
            .join("read-validation.md")
            .to_string_lossy(),
    )?;

    let manifest = instance.path().join("manifest.txt");
    fs::remove_file(&manifest)?;

    for args in [
        ["memory", "candidates", "-i", ip.as_ref()],
        ["task", "show", "-i", ip.as_ref()],
    ] {
        let error = assert_err(&args)?;
        assert!(
            error.contains("instance manifest is missing"),
            "read command unexpectedly bypassed missing manifest: {error}"
        );
    }

    fs::write(&manifest, "not a valid instance manifest\n")?;
    for args in [
        ["memory", "candidates", "-i", ip.as_ref()],
        ["task", "show", "-i", ip.as_ref()],
    ] {
        let error = assert_err(&args)?;
        assert!(
            error.contains("parse instance manifest"),
            "read command unexpectedly bypassed invalid manifest: {error}"
        );
    }

    Ok(())
}
