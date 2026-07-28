use super::test_helpers::{adapter, shell_request};
use super::*;

#[tokio::test]
async fn rejects_working_directory_outside_readable_roots() -> Result<(), Box<dyn std::error::Error>>
{
    let readable_root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let mut request = shell_request("pwd", 5000);
    request.working_directory = outside.path().to_path_buf();
    request.readable_roots = vec![readable_root.path().to_path_buf()];

    let result = adapter().execute(request).await;

    assert!(
        matches!(
            result,
            Err(PortError::InvalidInputContext {
                context: "working directory outside readable roots",
                ..
            })
        ),
        "expected typed cwd containment error, got {result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn rejects_working_directory_under_normalized_blocked_path()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested)?;
    let mut request = shell_request("pwd", 5000);
    request.working_directory = nested;
    request.readable_roots = vec![root.path().to_path_buf()];
    request.blocked_paths = vec![root.path().join("nested").join("..")];

    let result = adapter().execute(request).await;

    assert!(
        matches!(
            result,
            Err(PortError::InvalidInputContext {
                context: "working directory blocked by exclusion",
                ..
            })
        ),
        "expected typed blocked cwd error, got {result:?}"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_working_directory_under_symlinked_blocked_path()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir()?;
    let blocked_target = root.path().join("blocked-target");
    std::fs::create_dir(&blocked_target)?;
    let blocked_alias = root.path().join("blocked-alias");
    symlink(&blocked_target, &blocked_alias)?;

    let mut request = shell_request("pwd", 5000);
    request.working_directory = blocked_target;
    request.readable_roots = vec![root.path().to_path_buf()];
    request.blocked_paths = vec![blocked_alias];

    let result = adapter().execute(request).await;

    assert!(
        matches!(
            result,
            Err(PortError::InvalidInputContext {
                context: "working directory blocked by exclusion",
                ..
            })
        ),
        "expected typed symlink-blocked cwd error, got {result:?}"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn hostile_path_cannot_replace_allowlisted_command() -> Result<(), Box<dyn std::error::Error>>
{
    use std::env;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    static PATH_LOCK: Mutex<()> = Mutex::new(());
    let handle = tokio::runtime::Handle::current();
    let outcome = tokio::task::spawn_blocking(move || -> Result<_, std::io::Error> {
        let _path_lock = PATH_LOCK
            .lock()
            .map_err(|_| std::io::Error::other("PATH test lock poisoned"))?;
        let hostile_dir = tempfile::tempdir()?;
        let hostile_echo = hostile_dir.path().join("echo");
        std::fs::write(&hostile_echo, b"#!/bin/sh\nprintf 'hijacked\\n'\n")?;
        let mut permissions = std::fs::metadata(&hostile_echo)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hostile_echo, permissions)?;

        let previous_path = env::var_os("PATH");
        unsafe {
            env::set_var("PATH", hostile_dir.path());
        }
        let result = handle.block_on(adapter().execute(shell_request("echo trusted", 5000)));
        match previous_path {
            Some(path) => unsafe { env::set_var("PATH", path) },
            None => unsafe { env::remove_var("PATH") },
        }

        result.map_err(|error| std::io::Error::other(error.to_string()))
    })
    .await??;

    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.stdout, b"trusted\n");
    Ok(())
}
