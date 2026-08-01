#![forbid(unsafe_code)]

//! Integration-test fixture helpers for the maestria CLI.
//!
//! The CLI crate is a binary; integration-test crates compile this library
//! target so fixture helpers can be shared without per-binary dead-code
//! warnings. Production code never uses this module.
//!
//! Responsibility map:
//! - `test_support`: CLI integration-test fixture helpers.

pub mod test_support;
