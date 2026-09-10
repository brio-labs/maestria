use maestria_ports::MultiVectorSourceIdentity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EncodeRequest<'a> {
    pub schema_version: u16,
    pub model: &'a str,
    pub fingerprint: &'a str,
    /// Profile-level binding understood by the sidecar; `identity` remains
    /// the complete generation/realm/trust identity from the Rust contract.
    pub profile_identity: &'a str,
    pub identity: &'a str,
    pub kind: WireInputKind,
    pub input_hash: &'a str,
    pub text: &'a str,
    pub max_vectors: u32,
    pub remaining_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'a MultiVectorSourceIdentity>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireInputKind {
    Query,
    Document,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EncodeResponse {
    pub schema_version: u16,
    pub model: String,
    pub fingerprint: String,
    pub profile_identity: String,
    pub identity: String,
    pub input_hash: String,
    #[serde(default)]
    pub source: Option<MultiVectorSourceIdentity>,
    pub original_token_count: u32,
    pub truncated: bool,
    pub vectors: Vec<WireToken>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireToken {
    pub position: u32,
    pub embedding: Vec<f32>,
}
