use super::test_helpers::{adapter, shell_request};
use maestria_ports::{HarnessAdapter, PortError};

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
async fn stable_in_root_symlink_reads_target() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir()?;
    let target = root.path().join("target.txt");
    let alias = root.path().join("alias.txt");
    std::fs::write(&target, b"inside\n")?;
    symlink("target.txt", &alias)?;

    let mut request = shell_request("cat alias.txt", 5000);
    request.working_directory = root.path().to_path_buf();
    request.readable_roots = vec![root.path().to_path_buf()];
    let outcome = adapter().execute(request).await?;
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.stdout, b"inside\n");
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn escaping_symlink_is_rejected_before_output() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let outside_file = outside.path().join("outside.txt");
    let alias = root.path().join("alias.txt");
    std::fs::write(&outside_file, b"outside-secret\n")?;
    symlink(&outside_file, &alias)?;

    let mut request = shell_request("cat alias.txt", 5000);
    request.working_directory = root.path().to_path_buf();
    request.readable_roots = vec![root.path().to_path_buf()];
    let result = adapter().execute(request).await;
    assert!(matches!(result, Err(PortError::InvalidInputContext { .. })));
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ambient_environment_file_is_not_readable() -> Result<(), Box<dyn std::error::Error>> {
    let mut request = shell_request("cat /proc/self/environ", 5000);
    request.readable_roots = vec![std::path::PathBuf::from("/")];
    let result = adapter().execute(request).await;
    assert!(
        matches!(
            result,
            Err(PortError::InvalidInputContext {
                context: "cat environment disclosure denied",
                ..
            })
        ),
        "ambient environment must not be exposed: {result:?}"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn readable_root_handle_survives_path_replacement() -> Result<(), Box<dyn std::error::Error>>
{
    let parent = tempfile::tempdir()?;
    let root = parent.path().join("root");
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("allowed.txt"), b"pinned\n")?;

    let mut request = shell_request("cat allowed.txt", 5000);
    request.working_directory = root.clone();
    request.readable_roots = vec![root.clone()];
    let authorization = crate::command::authorize_paths(&request)?;

    let moved = parent.path().join("moved-root");
    std::fs::rename(&root, &moved)?;
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("allowed.txt"), b"replacement\n")?;

    let (exit_code, stdout, stderr) = crate::process::execute_command(
        "cat",
        &["allowed.txt".to_string()],
        &request,
        &authorization,
    )
    .await?;
    assert_eq!(exit_code, 0);
    assert_eq!(stdout, b"pinned\n");
    assert!(stderr.is_empty());
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn blocked_path_remains_bound_after_root_rename() -> Result<(), Box<dyn std::error::Error>> {
    let parent = tempfile::tempdir()?;
    let root = parent.path().join("root");
    let private = root.join("private");
    std::fs::create_dir_all(&private)?;
    std::fs::write(private.join("secret.txt"), b"secret\n")?;

    let mut request = shell_request("cat private/secret.txt", 5000);
    request.working_directory = root.clone();
    request.readable_roots = vec![root.clone()];
    request.blocked_paths = vec![private];
    let authorization = crate::command::authorize_paths(&request)?;

    let moved = parent.path().join("moved-root");
    std::fs::rename(&root, &moved)?;
    std::fs::create_dir(&root)?;

    let result = crate::process::execute_command(
        "cat",
        &["private/secret.txt".to_string()],
        &request,
        &authorization,
    )
    .await;
    assert!(matches!(result, Err(PortError::InvalidInputContext { .. })));
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn blocked_path_follows_root_relocated_beneath_another_root()
-> Result<(), Box<dyn std::error::Error>> {
    let parent = tempfile::tempdir()?;
    let root_a = parent.path().join("root-a");
    let root_b = parent.path().join("root-b");
    let private = root_b.join("private");
    std::fs::create_dir(&root_a)?;
    std::fs::create_dir_all(&private)?;
    std::fs::write(private.join("secret.txt"), b"secret\n")?;

    let mut request = shell_request("cat moved-b/private/secret.txt", 5000);
    request.working_directory = root_a.clone();
    request.readable_roots = vec![root_a.clone(), root_b.clone()];
    request.blocked_paths = vec![private];
    let authorization = crate::command::authorize_paths(&request)?;

    std::fs::rename(&root_b, root_a.join("moved-b"))?;
    let result = crate::process::execute_command(
        "cat",
        &["moved-b/private/secret.txt".to_string()],
        &request,
        &authorization,
    )
    .await;
    assert!(matches!(result, Err(PortError::InvalidInputContext { .. })));
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn blocked_subtree_remains_blocked_after_rename() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let private = root.path().join("private");
    std::fs::create_dir(&private)?;
    std::fs::write(private.join("secret.txt"), b"secret\n")?;

    let mut request = shell_request("cat public/secret.txt", 5000);
    request.working_directory = root.path().to_path_buf();
    request.readable_roots = vec![root.path().to_path_buf()];
    request.blocked_paths = vec![private.clone()];
    let authorization = crate::command::authorize_paths(&request)?;

    std::fs::rename(&private, root.path().join("public"))?;
    let result = crate::process::execute_command(
        "cat",
        &["public/secret.txt".to_string()],
        &request,
        &authorization,
    )
    .await;
    assert!(matches!(result, Err(PortError::InvalidInputContext { .. })));
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn nonexistent_blocked_path_cannot_be_replaced_by_symlink()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir()?;
    let public = root.path().join("public");
    std::fs::create_dir(&public)?;
    std::fs::write(public.join("secret.txt"), b"secret\n")?;

    let mut request = shell_request("cat private/secret.txt", 5000);
    request.working_directory = root.path().to_path_buf();
    request.readable_roots = vec![root.path().to_path_buf()];
    request.blocked_paths = vec![root.path().join("private")];
    let authorization = crate::command::authorize_paths(&request)?;

    symlink("public", root.path().join("private"))?;
    let result = crate::process::execute_command(
        "cat",
        &["private/secret.txt".to_string()],
        &request,
        &authorization,
    )
    .await;
    assert!(matches!(result, Err(PortError::InvalidInputContext { .. })));
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn root_relocation_cannot_enter_absolute_blocked_path()
-> Result<(), Box<dyn std::error::Error>> {
    let parent = tempfile::tempdir()?;
    let root = parent.path().join("root");
    let private = root.join("private");
    std::fs::create_dir_all(&private)?;
    std::fs::write(private.join("secret.txt"), b"secret\n")?;
    let destination = parent.path().join("destination");

    let mut request = shell_request("cat private/secret.txt", 5000);
    request.working_directory = root.clone();
    request.readable_roots = vec![root.clone()];
    request.blocked_paths = vec![destination.join("private")];
    let authorization = crate::command::authorize_paths(&request)?;

    std::fs::rename(&root, &destination)?;
    let result = crate::process::execute_command(
        "cat",
        &["private/secret.txt".to_string()],
        &request,
        &authorization,
    )
    .await;
    assert!(matches!(result, Err(PortError::InvalidInputContext { .. })));
    Ok(())
}
