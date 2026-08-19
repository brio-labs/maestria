#![forbid(unsafe_code)]

//! Local-first orchestration services for Maestria.
//!
//! This crate composes port traits and domain-shaped values. It deliberately
//! avoids concrete SQL, filesystem, search-engine, and parser implementations.

/// Responsibility map:
/// - `error`: module responsibility.
/// - `evidence_opening`: module responsibility.
/// - `ingestion`: module responsibility.
/// - `instance`: module responsibility.
/// - `manifest`: module responsibility.
/// - `manifest_scope`: lexical normalization and blocked pattern matching.
/// - `metrics`: shared metric formatting helpers.
/// - `notebook_draft_opening`: validates and opens persisted notebook draft blobs.
/// - `ports`: module responsibility.
/// - `provenance`: module responsibility.
/// - `types`: module responsibility.
mod error;
mod evidence_opening;
mod ingestion;
mod instance;
mod manifest;
mod manifest_scope;
mod metrics;
mod notebook_draft_opening;
mod ports;
mod provenance;
mod types;

pub use error::{CoreError, CoreResult};
pub use ingestion::build_artifact_detected_input;
pub use instance::{InitInstanceInput, InitInstancePlan, InstanceLayout, InstanceService};
pub use manifest::{
    EmbeddingConfig, InstanceManifest, OcrConfig, SparseProfileConfig, VisualConfig,
};
pub use manifest_scope::{lexical_normalize, path_matches_pattern};
pub use metrics::{format_duration, rate_per_second};
pub use notebook_draft_opening::open_notebook_draft_body;
pub use ports::{CorePorts, CoreServices};
pub use provenance::{artifact_id_for, artifact_id_for_content_hash, content_hash};
pub use types::{
    OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput, SourceGroundedCardHit,
    SourceGroundedSearchHit,
};
