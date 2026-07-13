// ── Kernel safety and dependency invariants ───────────────────────

#[test]
fn kernel_does_not_depend_on_forbidden_runtime_crates_or_operators() {
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"));
    let prelude = source
        .split_once("#[cfg(test)]")
        .map_or(source, |(head, _)| head);
    let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));

    for forbidden in ["tokio", "sqlx", "reqwest", "tantivy", "axum"] {
        assert!(
            !manifest.contains(&format!("{forbidden} =")),
            "found forbidden runtime dependency token: {forbidden}"
        );
        assert!(
            !prelude.contains(forbidden),
            "found forbidden runtime token in source: {forbidden}"
        );
    }

    for forbidden in [
        "unwrap(",
        "expect(",
        "panic!(",
        "unreachable!(",
        "todo!(",
        "unimplemented!(",
    ] {
        assert!(
            !prelude.contains(forbidden),
            "found forbidden failure path token: {forbidden}"
        );
    }
}
