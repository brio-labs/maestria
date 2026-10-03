#[macro_use]
#[path = "payloads_convert.rs"]
pub(crate) mod convert;

#[path = "event_payloads.rs"]
pub(crate) mod event_payloads;
#[path = "ocr_event_payloads.rs"]
pub(crate) mod ocr_event_payloads;

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

#[path = "realm_read_grant_event_payloads.rs"]
pub(crate) mod realm_read_grant_event_payloads;

#[path = "misc_event_payloads.rs"]
pub(crate) mod misc_event_payloads;
#[path = "notebook_event_payloads.rs"]
pub(crate) mod notebook_event_payloads;

#[path = "provenance_payloads.rs"]
pub(crate) mod provenance_payloads;

#[path = "stored_security.rs"]
pub(crate) mod stored_security;

#[path = "stored_content.rs"]
pub(crate) mod stored_content;

#[path = "stored_generations.rs"]
pub(crate) mod stored_generations;

#[path = "stored_structure.rs"]
pub(crate) mod stored_structure;

#[path = "stored_model_agent.rs"]
pub(crate) mod stored_model_agent;

#[path = "stored_model_agent_stages.rs"]
pub(crate) mod stored_model_agent_stages;

#[path = "stored_evidence_pack.rs"]
pub(crate) mod stored_evidence_pack;

#[path = "stored_evidence_pack_reproducibility.rs"]
pub(crate) mod stored_evidence_pack_reproducibility;

#[path = "stored_evidence_pack_coverage.rs"]
pub(crate) mod stored_evidence_pack_coverage;

#[path = "stored_search.rs"]
pub(crate) mod stored_search;

#[path = "stored_search_plan.rs"]
pub(crate) mod stored_search_plan;

#[path = "stored_search_route.rs"]
pub(crate) mod stored_search_route;

#[path = "stored_search_plan_policy.rs"]
pub(crate) mod stored_search_plan_policy;

#[path = "stored_search_plan_requirements.rs"]
pub(crate) mod stored_search_plan_requirements;

#[path = "stored_search_candidate.rs"]
pub(crate) mod stored_search_candidate;

#[path = "stored_search_scores.rs"]
pub(crate) mod stored_search_scores;

#[path = "stored_search_outcome.rs"]
pub(crate) mod stored_search_outcome;

#[path = "stored_search_trace.rs"]
pub(crate) mod stored_search_trace;

#[path = "stored_search_expansion.rs"]
pub(crate) mod stored_search_expansion;

#[path = "stored_search_trace_lane.rs"]
pub(crate) mod stored_search_trace_lane;

#[path = "stored_search_trace_rerank.rs"]
pub(crate) mod stored_search_trace_rerank;

#[path = "stored_search_trace_diversity.rs"]
pub(crate) mod stored_search_trace_diversity;

#[path = "stored_search_trace_rewrite.rs"]
pub(crate) mod stored_search_trace_rewrite;

pub(crate) use provenance_payloads::{StoredParseStatus, StoredSourceSpan};

pub(crate) use event_payloads::StoredEventPayload;
pub(crate) use evidence_payloads::StoredEvidenceKind;

/// Current stored-event payload format version.
///
/// Rows write v5; tagged payload decoding remains compatible with records
/// written by prior versions.
pub(crate) const CURRENT_PAYLOAD_VERSION: i64 = 8;
