#![forbid(unsafe_code)]

//! Local HTTP adapter for the provider-neutral late-interaction contract.
//!
//! The ONNX runtime remains in the supervised Python sidecar. This crate only
//! validates identity-bound wire data and exposes the ports provider.
//! Responsibility map:
//! - `dto`: typed local provider wire protocol.
//! - `late_provider`: identity-bound local HTTP provider.

mod dto;
mod late_provider;

pub use late_provider::LocalHttpLateInteractionProvider;
