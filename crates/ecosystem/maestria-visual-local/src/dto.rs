use maestria_ports::{EmbeddingIdentity, VisualSource};
use serde::{Deserialize, Serialize};

/// Wire payload for a visual embedding request.
#[derive(Debug, Serialize)]
pub(super) struct VisualEmbeddingPayload {
    pub(super) model: String,
    pub(super) input: VisualInput,
    #[serde(skip)]
    pub(super) identity: EmbeddingIdentity,
}

/// Untagged wire input: a query string or a base64-encoded visual source.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum VisualInput {
    Text(String),
    Source {
        source: VisualSourcePayload,
        bytes: String,
    },
}

/// Wire description of a visual source for the embedding contract.
#[derive(Debug, Serialize)]
pub(super) struct VisualSourcePayload {
    pub(super) kind: &'static str,
    pub(super) blob: String,
    pub(super) page_start: Option<u32>,
    pub(super) page_end: Option<u32>,
    pub(super) page: Option<u32>,
    pub(super) x: Option<u32>,
    pub(super) y: Option<u32>,
    pub(super) width: Option<u32>,
    pub(super) height: Option<u32>,
}

/// Wire response for a visual embedding request.
#[derive(Debug, Deserialize)]
pub(super) struct VisualApiResponse {
    pub(super) data: Vec<VisualData>,
    #[serde(default)]
    pub(super) model: String,
}

/// One embedding entry in a visual API response.
#[derive(Debug, Deserialize)]
pub(super) struct VisualData {
    pub(super) embedding: Vec<f32>,
}

/// Converts a port `VisualSource` into its wire representation.
pub(super) fn source_payload(source: &VisualSource) -> VisualSourcePayload {
    match source {
        VisualSource::Page {
            blob,
            page_start,
            page_end,
        } => VisualSourcePayload {
            kind: "page",
            blob: blob.to_string(),
            page_start: Some(*page_start),
            page_end: Some(*page_end),
            page: None,
            x: None,
            y: None,
            width: None,
            height: None,
        },
        VisualSource::Region {
            blob,
            page,
            x,
            y,
            width,
            height,
        } => VisualSourcePayload {
            kind: "region",
            blob: blob.to_string(),
            page_start: None,
            page_end: None,
            page: Some(*page),
            x: Some(*x),
            y: Some(*y),
            width: Some(*width),
            height: Some(*height),
        },
    }
}
