use super::test_helpers::{adapter, shell_request};
use super::*;
use std::path::PathBuf;

#[tokio::test]
async fn cat_rejects_path_outside_readable_roots() -> Result<(), Box<dyn std::error::Error>> {
    let mut req = shell_request("cat /etc/hostname", 5000);
    req.readable_roots = vec![PathBuf::from("/tmp")];
    let result = adapter().execute(req).await;
    assert!(
        matches!(result, Err(PortError::InvalidInputContext { .. })),
        "expected InvalidInput for path outside roots, got {result:?}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn cat_validation_returns_canonical_existing_operand() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let temp_dir = tempfile::tempdir()?;
    let target = temp_dir.path().join("target.txt");
    let alias = temp_dir.path().join("alias.txt");
    std::fs::write(&target, b"validated")?;
    symlink(&target, &alias)?;

    let mut request = shell_request("cat alias.txt", 5000);
    request.working_directory = temp_dir.path().to_path_buf();
    request.readable_roots = vec![temp_dir.path().to_path_buf()];
    let argv = vec!["cat".to_string(), "alias.txt".to_string()];

    let validated = super::command::validate_cat_args("cat", &argv, &request)?;

    assert_eq!(validated, vec![target.to_string_lossy().into_owned()]);
    Ok(())
}
