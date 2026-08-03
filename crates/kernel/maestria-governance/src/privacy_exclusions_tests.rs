use super::*;
use std::path::Path;

#[test]
fn empty_exclusions_never_match() {
    let exclusions = PrivacyExclusions::new();
    assert!(!exclusions.is_excluded(Path::new("/home/user/.env")));
    assert!(!exclusions.is_excluded(Path::new("secret.key")));
}

#[test]
fn default_excludes_sensitive_names() {
    let exclusions = PrivacyExclusions::default();
    assert!(exclusions.is_excluded(Path::new("/src/.env")));
    assert!(exclusions.is_excluded(Path::new(".env")));
    assert!(exclusions.is_excluded(Path::new("/project/.git/config")));
    assert!(exclusions.is_excluded(Path::new("/etc/credentials")));
    assert!(exclusions.is_excluded(Path::new("secrets/db.yaml")));
    assert!(exclusions.is_excluded(Path::new("/home/user/.ssh/id_rsa")));
    assert!(exclusions.is_excluded(Path::new("/home/user/.ssh/authorized_keys")));
}

#[test]
fn default_excludes_sensitive_extensions() {
    let exclusions = PrivacyExclusions::default();
    assert!(exclusions.is_excluded(Path::new("/certs/server.pem")));
    assert!(exclusions.is_excluded(Path::new("tls.key")));
    assert!(exclusions.is_excluded(Path::new("/etc/ssl/bundle.pfx")));
    assert!(exclusions.is_excluded(Path::new("keystore.jks")));
    assert!(exclusions.is_excluded(Path::new("prod.env")));
}

#[test]
fn normal_paths_are_not_excluded() {
    let exclusions = PrivacyExclusions::default();
    assert!(!exclusions.is_excluded(Path::new("/src/main.rs")));
    assert!(!exclusions.is_excluded(Path::new("/docs/readme.md")));
    assert!(!exclusions.is_excluded(Path::new("Cargo.toml")));
    assert!(!exclusions.is_excluded(Path::new("/home/user/config.json")));
}

#[test]
fn custom_exclusions_work() {
    let exclusions = PrivacyExclusions::new()
        .with_name("classified")
        .with_extension("secret");
    assert!(exclusions.is_excluded(Path::new("/docs/classified/report.txt")));
    assert!(exclusions.is_excluded(Path::new("notes.secret")));
    assert!(!exclusions.is_excluded(Path::new("/docs/public/report.txt")));
}

#[test]
fn leading_dot_extension_is_normalized() {
    let exclusions = PrivacyExclusions::new().with_extension(".pem");
    assert!(exclusions.is_excluded(Path::new("/certs/server.pem")));
    assert_eq!(exclusions.sensitive_extensions(), ["pem"]);
}

#[test]
fn empty_path_is_not_excluded() {
    let exclusions = PrivacyExclusions::default();
    assert!(!exclusions.is_excluded(Path::new("")));
}
