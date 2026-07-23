use super::command::filename_matches;
use super::test_helpers::{adapter, shell_request};
use super::*;

#[test]
fn filename_matches_exact() -> Result<(), Box<dyn std::error::Error>> {
    assert!(filename_matches(".env", ".env"));
    assert!(!filename_matches(".env", "other"));
    Ok(())
}

#[test]
fn filename_matches_wildcard_suffix() -> Result<(), Box<dyn std::error::Error>> {
    assert!(filename_matches("secret.key", "*.key"));
    assert!(!filename_matches("key.txt", "*.key"));
    Ok(())
}

#[test]
fn filename_matches_wildcard_prefix() -> Result<(), Box<dyn std::error::Error>> {
    assert!(filename_matches(".env.prod", ".env.*"));
    assert!(!filename_matches(".env", ".env.*"));
    Ok(())
}

#[test]
fn filename_matches_question_wildcard() -> Result<(), Box<dyn std::error::Error>> {
    assert!(filename_matches("a.key", "?.key"));
    assert!(!filename_matches("ab.key", "?.key"));
    Ok(())
}

#[tokio::test]
async fn cat_rejects_blocked_pattern() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let keyfile = tmp.path().join("secret.key");
    std::fs::write(&keyfile, b"keydata")?;
    let mut req = shell_request(&format!("cat {}", keyfile.display()), 5000);
    req.readable_roots = vec![tmp.path().to_path_buf()];
    req.blocked_patterns = vec!["*.key".into()];
    assert!(adapter().execute(req).await.is_err());
    Ok(())
}

#[tokio::test]
async fn cat_rejects_dotenv_pattern() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let envfile = tmp.path().join(".env");
    std::fs::write(&envfile, b"SECRET=xyz")?;
    let mut req = shell_request(&format!("cat {}", envfile.display()), 5000);
    req.readable_roots = vec![tmp.path().to_path_buf()];
    req.blocked_patterns = vec![".env".into()];
    assert!(adapter().execute(req).await.is_err());
    Ok(())
}
