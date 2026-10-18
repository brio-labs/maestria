use super::*;

#[test]
fn secret_scan_classifies_without_retaining_values() {
    let scan =
        scan_secrets("password=super-secret-value\n-----BEGIN PRIVATE KEY-----\nAKIA1234567890");
    assert_eq!(scan.findings.len(), 3);
    assert_eq!(scan.findings[0].kind, SecretKind::CredentialAssignment);
    assert_eq!(scan.findings[0].line, 1);
    assert_eq!(scan.findings[1].kind, SecretKind::PrivateKey);
    assert_eq!(scan.findings[2].kind, SecretKind::AccessToken);
    assert!(!format!("{scan:?}").contains("super-secret-value"));
}

#[test]
fn secret_scan_detects_exported_credentials() {
    let scan = scan_secrets("export API_KEY = value\nexported_token = prose");
    assert_eq!(scan.findings.len(), 1);
    assert_eq!(scan.findings[0].kind, SecretKind::CredentialAssignment);
}

#[test]
fn secret_scan_detects_structured_credentials() {
    let scan = scan_secrets("api_key: value\n{\"password\":\"value\"}");
    assert_eq!(scan.findings.len(), 2);
}

#[test]
fn secret_scan_allows_normal_text() {
    assert!(scan_secrets("passwords are rotated regularly").is_clean());
}
