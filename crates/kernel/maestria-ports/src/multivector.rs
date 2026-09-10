#[path = "multivector_identity.rs"]
mod identity;
#[cfg(test)]
#[path = "multivector_tests.rs"]
mod tests;
#[path = "multivector_values.rs"]
mod values;

pub use identity::{
    MultiVectorCompression, MultiVectorFingerprint, MultiVectorIdentity, MultiVectorInputKind,
    MultiVectorSimilarity, MultiVectorSourceIdentity, MultiVectorTokenId, MultiVectorTokenPolicy,
};
pub use values::{
    BoundedProviderTransport, LateInteractionContribution, LateInteractionProvider,
    LateInteractionScorer, LateInteractionScoringResult, MultiVectorDocument,
    MultiVectorDocumentRequest, MultiVectorQuery, MultiVectorQueryRequest, MultiVectorToken,
    ProviderCallControl, contribution_order,
};

pub const MULTIVECTOR_TEXT_V1: &str = "multivector_text_v1";
pub const LATE_INTERACTION_SCORE_SCALE_V1: &str = "late_interaction_maxsim_micros_v1";
pub const LATE_INTERACTION_SCORE_DENOMINATOR: i64 = 1_000_000;
pub const DEFAULT_MAX_QUERY_VECTORS: usize = 128;
pub const DEFAULT_MAX_DOCUMENT_VECTORS: usize = 512;
pub const DEFAULT_MAX_CONTRIBUTIONS: usize = 16;
pub const MAX_MULTIVECTOR_DIMENSIONS: usize = 4096;
