//! Minimal governance crate placeholder.

pub const GOVERNANCE_VERSION: &str = "0.1.0";

#[cfg(test)]
mod tests {
    #[test]
    fn governance_version_is_set() {
        assert!(!super::GOVERNANCE_VERSION.is_empty());
    }
}
