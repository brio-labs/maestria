use super::*;

// ── lexical_normalize ────────────────────────────────────────

#[test]
fn normalize_absolute_no_ops() {
    assert_eq!(
        lexical_normalize(Path::new("/a/b/c")),
        Some(PathBuf::from("/a/b/c"))
    );
}

#[test]
fn normalize_resolves_current_dir() {
    assert_eq!(
        lexical_normalize(Path::new("/a/./b/./c")),
        Some(PathBuf::from("/a/b/c"))
    );
}

#[test]
fn normalize_resolves_parent_dir() {
    assert_eq!(
        lexical_normalize(Path::new("/a/b/../c")),
        Some(PathBuf::from("/a/c"))
    );
}

#[test]
fn normalize_multiple_parent_dirs() {
    assert_eq!(
        lexical_normalize(Path::new("/a/b/c/../../d")),
        Some(PathBuf::from("/a/d"))
    );
}

#[test]
fn normalize_relative_path() {
    assert_eq!(
        lexical_normalize(Path::new("a/b/c")),
        Some(PathBuf::from("a/b/c"))
    );
}

#[test]
fn normalize_relative_with_parent_dir() {
    assert_eq!(
        lexical_normalize(Path::new("a/b/../c")),
        Some(PathBuf::from("a/c"))
    );
}

#[test]
fn normalize_escapes_root() {
    assert_eq!(lexical_normalize(Path::new("/a/../..")), None);
    assert_eq!(lexical_normalize(Path::new("/..")), None);
    assert_eq!(lexical_normalize(Path::new("/a/b/../../..")), None);
}

#[test]
fn normalize_relative_starting_with_parent() {
    // "../foo" — starts with ParentDir, cannot resolve
    assert_eq!(lexical_normalize(Path::new("../foo")), None);
}

#[test]
fn normalize_empty_path() {
    assert_eq!(lexical_normalize(Path::new("")), None);
}

#[test]
fn normalize_only_dots() {
    // Just "." components on a relative path — resolves to nothing
    assert_eq!(lexical_normalize(Path::new("./.")), None);
}

// ── check_containment ────────────────────────────────────────

fn roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/home/user/project")]
}

#[test]
fn containment_ok_nested_path() {
    assert_eq!(
        check_containment(&roots(), Path::new("/home/user/project/src/main.rs")),
        Ok(())
    );
}

#[test]
fn containment_ok_root_itself() {
    assert_eq!(
        check_containment(&roots(), Path::new("/home/user/project")),
        Ok(())
    );
}

#[test]
fn containment_ok_with_dot_components() {
    assert_eq!(
        check_containment(&roots(), Path::new("/home/user/project/./src/./main.rs")),
        Ok(())
    );
}

#[test]
fn containment_ok_with_valid_parent_dirs() {
    assert_eq!(
        check_containment(&roots(), Path::new("/home/user/project/src/../lib/util.rs")),
        Ok(())
    );
}

#[test]
fn containment_rejects_empty_path() {
    assert_eq!(
        check_containment(&roots(), Path::new("")),
        Err(ContainmentError::EmptyPath)
    );
}

#[test]
fn containment_rejects_escape() {
    assert_eq!(
        check_containment(&roots(), Path::new("/home/user/project/../../etc/passwd")),
        Err(ContainmentError::PathNotUnderAnyRoot {
            path: PathBuf::from("/home/etc/passwd"),
        })
    );
}

#[test]
fn containment_rejects_root_escape() {
    assert_eq!(
        check_containment(&roots(), Path::new("/home/user/project/../../../..")),
        Err(ContainmentError::PathEscapesRoot {
            path: PathBuf::from("/home/user/project/../../../.."),
        })
    );
}

#[test]
fn containment_rejects_relative_starting_with_parent() {
    assert_eq!(
        check_containment(&roots(), Path::new("../secret")),
        Err(ContainmentError::PathEscapesRoot {
            path: PathBuf::from("../secret"),
        })
    );
}

#[test]
fn containment_rejects_path_outside_all_roots() {
    assert_eq!(
        check_containment(&roots(), Path::new("/other/project/file.rs")),
        Err(ContainmentError::PathNotUnderAnyRoot {
            path: PathBuf::from("/other/project/file.rs"),
        })
    );
}

#[test]
fn containment_matches_any_root() {
    let multi_roots = vec![
        PathBuf::from("/home/user/project"),
        PathBuf::from("/opt/shared"),
    ];
    assert_eq!(
        check_containment(&multi_roots, Path::new("/opt/shared/lib/util.rs")),
        Ok(())
    );
}

#[test]
fn containment_requires_normalized_root_match() {
    // Root with `..` that normalises to the actual root.
    let tricky_roots = vec![PathBuf::from("/home/user/project/sub/..")];
    // Normalised root → /home/user/project
    assert_eq!(
        check_containment(&tricky_roots, Path::new("/home/user/project/src/main.rs")),
        Ok(())
    );
}

// ── Scope containment methods ────────────────────────────────

fn sample_scope() -> Scope {
    Scope::new(
        vec![PathBuf::from("/allowed/read")],
        vec![PathBuf::from("/allowed/write")],
        vec!["shell".into()],
        vec!["rm -rf".into()],
        true,
    )
}

#[test]
fn scope_check_read_containment_ok() {
    let scope = sample_scope();
    assert_eq!(
        scope.check_read_containment(Path::new("/allowed/read/docs/note.md")),
        Ok(())
    );
    // Also allowed via write root
    assert_eq!(
        scope.check_read_containment(Path::new("/allowed/write/output.md")),
        Ok(())
    );
}

#[test]
fn scope_check_read_containment_rejects_outside() {
    let scope = sample_scope();
    assert!(
        scope
            .check_read_containment(Path::new("/other/place/file.txt"))
            .is_err()
    );
}

#[test]
fn scope_check_read_containment_rejects_escape() {
    let scope = sample_scope();
    assert!(
        scope
            .check_read_containment(Path::new("/allowed/read/../.."))
            .is_err()
    );
}

#[test]
fn scope_check_write_containment_ok() {
    let scope = sample_scope();
    assert_eq!(
        scope.check_write_containment(Path::new("/allowed/write/output.md")),
        Ok(())
    );
}

#[test]
fn scope_check_write_containment_rejects_read_only() {
    let scope = sample_scope();
    assert!(
        scope
            .check_write_containment(Path::new("/allowed/read/docs/note.md"))
            .is_err()
    );
}

// ── ScopeGuard delegation ────────────────────────────────────

#[test]
fn guard_delegates_containment() {
    let scope = sample_scope();
    let guard = ScopeGuard::new(scope);
    assert_eq!(
        guard.check_read_containment(Path::new("/allowed/read/docs/note.md")),
        Ok(())
    );
}
