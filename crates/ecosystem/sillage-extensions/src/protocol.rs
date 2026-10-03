mod capabilities;
mod messages;
mod views;

pub use capabilities::{
    CapabilityError, CapabilityFailure, CapabilityRequest, CapabilityResponse, CapabilitySuccess,
    FileSearchResult, HttpMethod, OpenRequestTarget, StorageOperation, validate_capability_request,
};
pub use messages::{HostMessage, ProtocolError, WorkerMessage};
pub use views::{
    Action, ActionRole, DetailBlock, FormField, FormValue, FormValues, ListItem, PropertyRow,
    SelectChoice, View, validate_view,
};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_JSON_LINE_BYTES: usize = 262_144;
pub const MAX_CAPABILITY_RESPONSE_BYTES: usize = 65_536;
pub const MAX_ACTIVE_REQUESTS_PER_COMMAND: usize = 32;
