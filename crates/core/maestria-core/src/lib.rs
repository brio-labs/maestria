#![forbid(unsafe_code)]

//! Local-first orchestration services for Maestria.
//!
//! This crate composes port traits and domain-shaped values. It deliberately
//! avoids concrete SQL, filesystem, search-engine, and parser implementations.

mod error;
mod generation_gate;
mod graph_retrieval;
mod hierarchy_expansion;
mod ingestion;
mod instance;
mod lane_fusion;
mod lexical_helpers;
mod manifest;
mod ports;
mod provenance;
mod rank_fusion;
mod retrieval;
mod retrieval_lanes;
mod types;

pub const CORE_VERSION: &str = "0.1.0";

pub use error::{CoreError, CoreResult};
pub use ingestion::build_artifact_detected_input;
pub use instance::{InitInstanceInput, InitInstancePlan, InstanceLayout, InstanceService};
pub use manifest::InstanceManifest;
pub use ports::{CorePorts, CoreServices};
pub use provenance::artifact_id_for;
pub use provenance::content_hash;
pub use types::{
    EvidencePack, GraphConfig, HybridExecutionPolicy, HybridPromotionRecord,
    OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput, RetrievalLaneReport,
    RetrievalLaneStatus, RetrievalMode, SearchInput, SearchOutput, SourceGroundedCardHit,
    SourceGroundedSearchHit,
};
