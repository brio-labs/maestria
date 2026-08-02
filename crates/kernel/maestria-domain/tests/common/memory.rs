#[path = "evidence.rs"]
mod evidence;
#[path = "file_evidence.rs"]
mod file_evidence;

pub use evidence::register_artifact_and_claim;
pub use file_evidence::file_span_kind;
