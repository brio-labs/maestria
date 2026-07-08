//! Initial Maestria crate.

//! Generated during bootstrap. Keep this crate intentionally minimal.

pub const MAESTRIA_VERSION: &str = "0.1.0";

#[cfg(test)]
mod tests {
    use super::MAESTRIA_VERSION;

    #[test]
    fn exposes_version() {
        assert!(!MAESTRIA_VERSION.is_empty());
    }
}
