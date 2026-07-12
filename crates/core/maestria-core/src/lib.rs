#![forbid(unsafe_code)]

//! Local-first orchestration services for Maestria.
//!
//! This crate composes port traits and domain-shaped values. It deliberately
//! avoids concrete SQL, filesystem, search-engine, and parser implementations.

mod error;
mod ingestion;
mod instance;
mod manifest;
mod ports;
mod provenance;
mod recovery;
mod retrieval;
mod types;

pub const CORE_VERSION: &str = "0.1.0";

pub use error::{CoreError, CoreResult};
pub use instance::{InitInstanceInput, InitInstancePlan, InstanceLayout, InstanceService};
pub use manifest::InstanceManifest;
pub use ports::{CorePorts, CoreServices};
pub use types::{
    IngestFileInput, IngestFileOutput, OpenChunkEvidenceInput, OpenEvidenceInput,
    OpenEvidenceOutput, SearchInput, SearchOutput, SourceGroundedSearchHit,
};
pub use provenance::artifact_id_for;
pub use provenance::content_hash;
