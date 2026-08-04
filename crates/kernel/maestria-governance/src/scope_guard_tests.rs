use super::*;

#[test]
fn scope_guard_checks_read_write_paths() {
    let scope = Scope::new(
        vec![std::path::PathBuf::from("/allowed/read")],
        vec![std::path::PathBuf::from("/allowed/write")],
        vec!["shell".into()],
        vec!["rm -rf".into()],
        true,
    );
    let guard = ScopeGuard::new(scope);

    assert!(
        guard
            .check_read_containment(std::path::Path::new("/allowed/read/docs/note.md"))
            .is_ok()
    );
    assert!(
        guard
            .check_write_containment(std::path::Path::new("/allowed/write/output.md"))
            .is_ok()
    );
    assert!(
        guard
            .check_write_containment(std::path::Path::new("/allowed/read/docs/note.md"))
            .is_err()
    );
    assert!(!guard.command_allowed("rm -rf /tmp"));
    assert!(guard.harness_allowed("shell"));
    assert!(guard.web_allowed());
}
