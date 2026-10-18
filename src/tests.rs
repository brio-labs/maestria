use super::SILLAGE_VERSION;

#[test]
fn exposes_version() {
    assert!(!SILLAGE_VERSION.is_empty());
}
