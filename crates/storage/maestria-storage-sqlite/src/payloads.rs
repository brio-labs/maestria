#[path = "event_payloads.rs"]
pub(crate) mod event_payloads;

#[path = "legacy_payloads.rs"]
pub(crate) mod legacy_payloads;

#[path = "relation_payloads.rs"]
pub(crate) mod relation_payloads;

#[path = "evidence_payloads.rs"]
pub(crate) mod evidence_payloads;

#[path = "web_evidence_payload.rs"]
pub(crate) mod web_evidence_payload;

#[path = "artifact_event_payloads.rs"]
pub(crate) mod artifact_event_payloads;

#[path = "task_event_payloads.rs"]
pub(crate) mod task_event_payloads;

#[path = "claim_event_payloads.rs"]
pub(crate) mod claim_event_payloads;

#[path = "memory_event_payloads.rs"]
pub(crate) mod memory_event_payloads;

#[path = "misc_event_payloads.rs"]
pub(crate) mod misc_event_payloads;

#[path = "provenance_payloads.rs"]
pub(crate) mod provenance_payloads;

pub(crate) use provenance_payloads::{
    StoredParseStatus, StoredParsedRepresentation, StoredSourceSpan, default_status_parsed,
};

pub(crate) use event_payloads::StoredEventPayload;
pub(crate) use evidence_payloads::StoredEvidenceKind;
pub(crate) use legacy_payloads::LegacyStoredEventPayload;
