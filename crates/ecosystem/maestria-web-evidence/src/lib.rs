#![forbid(unsafe_code)]

//! Synchronous web evidence adapter backed by `ureq`.
//!
//! The adapter validates URL schemes, hashes fetched bytes, and extracts only
//! source metadata from HTML. Runtime orchestration owns blob persistence,
//! security scanning, policy decisions, and domain evidence recording.
//!
//! Responsibility map:
//! - `web_fetcher`: synchronous web evidence adapter.

mod web_fetcher;
pub use web_fetcher::UreqWebFetcher;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
