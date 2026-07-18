#![forbid(unsafe_code)]

//! Local-first orchestration services for Maestria.
//!
//! This crate composes port traits and domain-shaped values. It deliberately
//! avoids concrete SQL, filesystem, search-engine, and parser implementations.

mod error;
mod evidence_opening;
mod evidence_pack_provenance;
mod ingestion;
mod instance;
mod manifest;
mod ports;
mod provenance;
mod types;

pub const CORE_VERSION: &str = "0.6.1";

pub use error::{CoreError, CoreResult};
pub use ingestion::build_artifact_detected_input;
pub use instance::{InitInstanceInput, InitInstancePlan, InstanceLayout, InstanceService};
pub use manifest::InstanceManifest;
pub use ports::{CorePorts, CoreServices};
pub use provenance::artifact_id_for;
pub use provenance::content_hash;
pub use types::{
    ClaimCoverageStatus, ClaimEvidenceCoverage, EvidenceFreshness, EvidencePack,
    EvidencePackCompression, EvidencePackError, EvidencePackMetadata, EvidencePackReplayKey,
    EvidencePackReproducibility, OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput,
    SourceGroundedSearchHit,
};
