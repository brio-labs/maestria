//! HTTP transport and model identity validation shared across model provider adapters.
//!
//! Responsibility map:
//! - `client`: `ureq`-backed JSON client implementing [`maestria_ports::ProviderTransport`].
//! - `helpers`: serialization, model identity, and downstream error helpers.

mod client;
mod helpers;

pub use client::UreqJsonClient;
pub use helpers::{post_json, post_json_to, require_identity_eq, validate_model_identity};
