//! Shared error type for test-support helpers.
//!
//! A concrete `Error + Send + Sync` type so callers can propagate failures
//! with `?` into `Box<dyn std::error::Error>`, `Box<dyn Error + Send + Sync>`,
//! or `anyhow::Error` return types without adapter shims.

use std::fmt;

/// Failure of a test-support helper (git invocation or tree copy).
#[derive(Debug)]
pub struct TestSupportError(String);

impl TestSupportError {
    /// Builds an error from a rendered message.
    pub fn new(message: String) -> Self {
        Self(message)
    }
}

impl fmt::Display for TestSupportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TestSupportError {}

impl From<std::io::Error> for TestSupportError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}
