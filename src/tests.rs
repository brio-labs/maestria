use super::MAESTRIA_VERSION;

#[test]
fn exposes_version() {
    assert!(!MAESTRIA_VERSION.is_empty());
}
