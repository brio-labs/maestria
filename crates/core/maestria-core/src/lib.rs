#![forbid(unsafe_code)]

//! Local-first orchestration services for Maestria.
//!
//! This crate composes port traits and domain-shaped values. It deliberately
//! avoids concrete SQL, filesystem, search-engine, and parser implementations.

/// Responsibility map:
/// - `error`: module responsibility.
/// - `evidence_opening`: module responsibility.
/// - `evidence_pack_provenance`: module responsibility.
/// - `ingestion`: module responsibility.
/// - `instance`: module responsibility.
/// - `notebook_draft_opening`: validates and opens persisted notebook draft blobs.
/// - `manifest`: module responsibility.
/// - `ports`: module responsibility.
/// - `provenance`: module responsibility.
/// - `types`: module responsibility.
/// - `version`: core version metadata.
mod error;
mod evidence_opening;
mod evidence_pack_provenance;
mod ingestion;
mod instance;
mod manifest;
mod notebook_draft_opening;
mod ports;
mod provenance;
mod types;

mod version;

pub use version::CORE_VERSION;

pub use error::{CoreError, CoreResult};
pub use ingestion::build_artifact_detected_input;
pub use instance::{InitInstanceInput, InitInstancePlan, InstanceLayout, InstanceService};
pub use manifest::{EmbeddingConfig, InstanceManifest, OcrConfig, SparseProfileConfig, VisualConfig};
pub use notebook_draft_opening::open_notebook_draft_body;
pub use ports::{CorePorts, CoreServices};
pub use provenance::artifact_id_for;
pub use provenance::content_hash;
pub use types::{
    ClaimCoverageStatus, ClaimEvidenceCoverage, EvidenceFreshness, EvidencePack,
    EvidencePackCompression, EvidencePackError, EvidencePackMetadata, EvidencePackReplayKey,
    EvidencePackReproducibility, OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput,
    SourceGroundedCardHit, SourceGroundedSearchHit,
};
