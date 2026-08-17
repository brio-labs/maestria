use super::*;
use crate::test_support::TempDir;
use maestria_domain::{ArtifactId, BlobId, ParserStarted};
use std::path::{Path, PathBuf};

fn write_manifest(dir: &Path, read_roots: &[&str]) -> std::io::Result<PathBuf> {
    let mut lines = vec![
        "schema_version=2".to_string(),
        format!("realm_id={}", maestria_test_support::realm_id_str(10)),
        format!("root={}", dir.display()),
    ];
    for root in read_roots {
        lines.push(format!("read_root={root}"));
    }
    lines.push("excluded_pattern=.env".to_string());
    lines.push("excluded_pattern=.env.*".to_string());
    lines.push("excluded_pattern=*.pem".to_string());
    lines.push("excluded_pattern=*.key".to_string());
    lines.push("excluded_pattern=.ssh".to_string());
    lines.push("excluded_pattern=.gnupg".to_string());
    lines.push("excluded_pattern=node_modules".to_string());
    lines.push("excluded_pattern=target".to_string());
    lines.push("excluded_pattern=dist".to_string());
    lines.push("excluded_pattern=build".to_string());
    lines.push("excluded_pattern=secrets".to_string());
    lines.push(String::new());
    let contents = lines.join("\n");
    let manifest_path = dir.join("manifest.txt");
    fs::write(&manifest_path, contents)?;
    Ok(manifest_path)
}

fn make_recovery(
    source_path: &str,
    artifact_id: u64,
    title: &str,
) -> Result<RecoveryInputs, Box<dyn std::error::Error>> {
    Ok(RecoveryInputs {
        resume_parsers: vec![DomainInput::ResumeParser(ParserStarted {
            artifact_id: ArtifactId::new(artifact_id),
            title: title.to_string(),
            source_path: source_path.to_string(),
            content_hash: maestria_test_support::content_hash(10)?,
            blob_id: BlobId::new(artifact_id),
        })],
        start_full_text: Vec::new(),
        run_validations: Vec::new(),
    })
}

#[test]
fn validate_recovery_scope_accepts_in_scope_source_paths() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = TempDir::create()?;
    let root_str = temp
        .path()
        .to_str()
        .ok_or_else(|| std::io::Error::other("path is not valid UTF-8"))?;
    write_manifest(temp.path(), &[root_str])?;
    let layout = InstanceLayout::for_root(temp.path());

    let in_scope_path = temp.path().join("notes.md");
    fs::write(&in_scope_path, "# test")?;

    let recovery = make_recovery(&in_scope_path.display().to_string(), 1, "notes.md")?;

    validate_recovery_scope(&layout, &recovery)?;
    Ok(())
}

#[test]
fn validate_recovery_scope_rejects_out_of_scope_source_path()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::create()?;
    let root_str = temp
        .path()
        .to_str()
        .ok_or_else(|| std::io::Error::other("path is not valid UTF-8"))?;
    write_manifest(temp.path(), &[root_str])?;
    let layout = InstanceLayout::for_root(temp.path());

    let recovery = make_recovery("/tmp/outside.md", 1, "outside.md")?;

    let result = validate_recovery_scope(&layout, &recovery);
    let error = result
        .err()
        .ok_or("out-of-scope source path should be rejected")?;
    let msg = format!("{error:#}");
    assert!(
        msg.contains("outside the instance manifest read scope"),
        "expected scope rejection for out-of-scope path, got: {msg}"
    );
    assert!(
        msg.contains("/tmp/outside.md"),
        "error should name the offending path, got: {msg}"
    );
    assert!(
        msg.contains("artifact 1 \"outside.md\""),
        "error should identify the artifact, got: {msg}"
    );
    Ok(())
}

#[test]
fn validate_recovery_scope_rejects_excluded_source_path() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = TempDir::create()?;
    let root_str = temp
        .path()
        .to_str()
        .ok_or_else(|| std::io::Error::other("path is not valid UTF-8"))?;
    write_manifest(temp.path(), &[root_str])?;
    let layout = InstanceLayout::for_root(temp.path());

    let excluded_path = temp.path().join(".env.local");
    let recovery = make_recovery(&excluded_path.display().to_string(), 2, ".env.local")?;

    let result = validate_recovery_scope(&layout, &recovery);
    let error = result
        .err()
        .ok_or("excluded source path should be rejected")?;
    let msg = format!("{error:#}");
    assert!(
        msg.contains("outside the instance manifest read scope") || msg.contains("excluded"),
        "expected exclusion rejection for .env.local, got: {msg}"
    );
    Ok(())
}

#[test]
fn validate_recovery_scope_rejects_git_config_by_privacy() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = TempDir::create()?;
    let root_str = temp
        .path()
        .to_str()
        .ok_or_else(|| std::io::Error::other("path is not valid UTF-8"))?;
    // manifest excludes .env / .env.* / *.pem / *.key / .ssh / .gnupg /
    // node_modules / target / dist / build / secrets — but NOT .git
    write_manifest(temp.path(), &[root_str])?;
    let layout = InstanceLayout::for_root(temp.path());

    let git_config = temp.path().join(".git/config");
    let recovery = make_recovery(&git_config.display().to_string(), 10, ".git/config")?;

    let result = validate_recovery_scope(&layout, &recovery);
    let error = result
        .err()
        .ok_or(".git/config should be rejected by privacy policy")?;
    let msg = format!("{error:#}");
    assert!(
        msg.contains("privacy policy"),
        "expected privacy-policy rejection for .git/config, got: {msg}"
    );
    assert!(
        msg.contains(".git/config"),
        "error should name the offending path, got: {msg}"
    );
    Ok(())
}

#[test]
fn validate_recovery_scope_rejects_credentials_by_privacy() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = TempDir::create()?;
    let root_str = temp
        .path()
        .to_str()
        .ok_or_else(|| std::io::Error::other("path is not valid UTF-8"))?;
    // manifest does not exclude 'credentials' as a pattern
    write_manifest(temp.path(), &[root_str])?;
    let layout = InstanceLayout::for_root(temp.path());

    let creds_path = temp.path().join("credentials/tokens.json");
    let recovery = make_recovery(&creds_path.display().to_string(), 11, "tokens.json")?;

    let result = validate_recovery_scope(&layout, &recovery);
    let error = result
        .err()
        .ok_or("credentials directory should be rejected by privacy policy")?;
    let msg = format!("{error:#}");
    assert!(
        msg.contains("privacy policy"),
        "expected privacy-policy rejection for credentials path, got: {msg}"
    );
    Ok(())
}

#[test]
fn validate_recovery_scope_rejects_secret_extension_by_privacy()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::create()?;
    let root_str = temp
        .path()
        .to_str()
        .ok_or_else(|| std::io::Error::other("path is not valid UTF-8"))?;
    // manifest *.pem and *.key patterns exist, but .pfx is not covered
    write_manifest(temp.path(), &[root_str])?;
    let layout = InstanceLayout::for_root(temp.path());

    for (ext, artifact_id) in [("pfx", 12), ("p12", 13), ("jks", 14), ("keystore", 15)] {
        let file_path = temp.path().join(format!("certs/bundle.{ext}"));
        let recovery = make_recovery(
            &file_path.display().to_string(),
            artifact_id,
            &format!("bundle.{ext}"),
        )?;

        let result = validate_recovery_scope(&layout, &recovery);
        let error = result.err().ok_or(format!(
            ".{ext} extension should be rejected by privacy policy"
        ))?;
        let msg = format!("{error:#}");
        assert!(
            msg.contains("privacy policy"),
            "expected privacy-policy rejection for .{ext}, got: {msg}"
        );
    }
    Ok(())
}

#[test]
fn validate_recovery_scope_rejects_env_extension_by_privacy()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::create()?;
    let root_str = temp
        .path()
        .to_str()
        .ok_or_else(|| std::io::Error::other("path is not valid UTF-8"))?;
    // manifest has .env and .env.* as patterns, but not *.env extension
    write_manifest(temp.path(), &[root_str])?;
    let layout = InstanceLayout::for_root(temp.path());

    let env_file = temp.path().join("config/app.env");
    let recovery = make_recovery(&env_file.display().to_string(), 16, "app.env")?;

    let result = validate_recovery_scope(&layout, &recovery);
    let error = result
        .err()
        .ok_or(".env extension should be rejected by privacy policy")?;
    let msg = format!("{error:#}");
    assert!(
        msg.contains("privacy policy"),
        "expected privacy-policy rejection for .env extension, got: {msg}"
    );
    Ok(())
}

#[test]
fn validate_recovery_scope_empty_recovery_always_passes() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = TempDir::create()?;
    write_manifest(temp.path(), &["/some/other/root"])?;
    let layout = InstanceLayout::for_root(temp.path());

    let recovery = RecoveryInputs {
        resume_parsers: Vec::new(),
        start_full_text: Vec::new(),
        run_validations: Vec::new(),
    };

    validate_recovery_scope(&layout, &recovery)?;
    Ok(())
}
