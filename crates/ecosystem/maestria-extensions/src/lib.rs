#![forbid(unsafe_code)]

//! Versioned Sillage extension contract shared by trusted host and isolated worker.
//! Parsing types is not a grant: the host validates bundles, permissions and
//! untrusted worker output again before presenting views or performing effects.
//!
//! Responsibility map:
//! - `bundle`: sealed package installation and durable extension lifecycle.
//! - `grants`: capability authorization at invocation boundaries.
//! - `manifest`: extension manifest model and validation.
//! - `protocol`: host-worker messages, schemas, and capability validation.

pub mod bundle;
mod grants;
mod manifest;
mod protocol;

pub use grants::{GrantError, InvocationOrigin, authorize};
pub use manifest::{
    Command, CopyFormat, Entrypoint, MAX_MANIFEST_BYTES, Manifest, ManifestError, OpenTarget,
    Permission, SDK_API_VERSION, StorageScope,
};
pub use protocol::{
    Action, ActionRole, CapabilityError, CapabilityFailure, CapabilityRequest, CapabilityResponse,
    CapabilitySuccess, DetailBlock, FileSearchResult, FormField, FormValue, FormValues,
    HostMessage, HttpMethod, ListItem, MAX_ACTIVE_REQUESTS_PER_COMMAND,
    MAX_CAPABILITY_RESPONSE_BYTES, MAX_JSON_LINE_BYTES, OpenRequestTarget, PROTOCOL_VERSION,
    PropertyRow, ProtocolError, SelectChoice, StorageOperation, View, WorkerMessage,
    validate_capability_request, validate_view,
};
