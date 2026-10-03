use serde::{Deserialize, Serialize};

/// Wire payload for a sparse encoding request.
#[derive(Debug, Serialize)]
pub(super) struct SparseEncodePayload {
    pub(super) text: String,
    pub(super) kind: SparseKindWire,
}

/// Wire payload for a batched sparse encoding request.
#[derive(Debug, Serialize)]
pub(super) struct SparseEncodeBatchPayload<'a> {
    #[serde(borrow)]
    pub(super) texts: &'a [String],
    pub(super) kind: SparseKindWire,
}

/// Wire response for a batched sparse encoding request.
#[derive(Debug, Deserialize)]
pub(super) struct SparseBatchApiResponse {
    pub(super) vectors: Vec<SparseApiResponse>,
}

/// Wire kind selector for a sparse encoding request.
#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum SparseKindWire {
    Query,
    Document,
}

/// Wire response for a sparse encoding request.
///
/// Unknown response fields (such as the sidecar's provenance `model` value)
/// are ignored by serde.
#[derive(Debug, Deserialize)]
pub(super) struct SparseApiResponse {
    pub(super) term_ids: Vec<u32>,
    pub(super) weights: Vec<f32>,
}
