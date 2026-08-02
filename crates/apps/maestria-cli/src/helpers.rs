//! Command-side support façade: one named responsibility per sibling module.
//!
//! Responsibility map:
//! - `instance_access`: instance layout and manifest validation.
//! - `state_polling`: database-busy retry and durable kernel-state polling.
//! - `index_files`: index-path traversal and privacy/symlink/type filtering.
//! - `evidence_format`: evidence source-label formatting.

pub(crate) mod evidence_format;
pub(crate) mod index_files;
pub(crate) mod instance_access;
pub(crate) mod state_polling;

pub(crate) use evidence_format::source_label;
pub(crate) use index_files::collect_index_files;
pub(crate) use instance_access::{ensure_instance, load_manifest, validated_instance};
pub(crate) use state_polling::{
    load_kernel_state_with_retry, retry_db_busy, wait_for_kernel_state,
};
