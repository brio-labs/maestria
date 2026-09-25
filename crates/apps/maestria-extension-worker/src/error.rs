use std::io;

use maestria_extensions::ProtocolError;
use thiserror::Error;

use crate::{arguments::ArgumentError, bundle::BundleError, input::InputError};

#[derive(Debug, Error)]
pub(crate) enum WorkerError {
    #[error(transparent)]
    Arguments(#[from] ArgumentError),
    #[error(transparent)]
    Bundle(#[from] BundleError),
    #[error(transparent)]
    Input(#[from] InputError),
    #[error("extension protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("embedded JavaScript runtime failed: {0}")]
    JavaScript(#[from] rquickjs::Error),
    #[error("JSON conversion failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O failed during {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("host closed stdin while the worker was active")]
    InputClosed,
    #[error("host cancelled the active command")]
    Cancelled,
    #[error("command deadline expired")]
    Deadline,
    #[error("command ID is not declared by the selected entrypoint")]
    UndeclaredCommand,
    #[error("selected entrypoint declares no commands")]
    EmptyEntrypoint,
    #[error("module default export must exactly match its declared command IDs")]
    InvalidEntrypointExports,
    #[error("extension capability request is malformed or invalid")]
    InvalidCapabilityRequest,
    #[error("host capability response does not match its outstanding request")]
    InvalidCapabilityResponse,
    #[error("extension returned an invalid view")]
    InvalidView,
    #[error("extension returned while capability requests were still outstanding")]
    OutstandingCapabilityRequests,
    #[error("extension capability request was abandoned before its response")]
    AbandonedCapabilityRequest,
    #[error("worker received a host message that is invalid for the active command")]
    UnexpectedHostMessage,
    #[error("worker capability request limit was exceeded")]
    CapabilityRequestLimit,
    #[error("worker capability request ID space was exhausted")]
    CapabilityRequestIdExhausted,
}

impl WorkerError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Arguments(_) => "invalid_arguments",
            Self::Bundle(_) => "bundle_error",
            Self::Input(InputError::Io(_)) => "io_error",
            Self::Input(_) => "invalid_host_message",
            Self::Protocol(_) => "protocol_error",
            Self::JavaScript(_) => "javascript_error",
            Self::Json(_) => "json_error",
            Self::Io { .. } => "io_error",
            Self::InputClosed => "input_closed",
            Self::Cancelled => "cancelled",
            Self::Deadline => "deadline_exceeded",
            Self::UndeclaredCommand => "undeclared_command",
            Self::EmptyEntrypoint => "empty_entrypoint",
            Self::InvalidEntrypointExports => "invalid_entrypoint_exports",
            Self::InvalidCapabilityRequest => "invalid_capability_request",
            Self::InvalidCapabilityResponse => "invalid_capability_response",
            Self::InvalidView => "invalid_view",
            Self::OutstandingCapabilityRequests => "outstanding_capability_requests",
            Self::AbandonedCapabilityRequest => "abandoned_capability_request",
            Self::UnexpectedHostMessage => "unexpected_host_message",
            Self::CapabilityRequestLimit => "capability_request_limit",
            Self::CapabilityRequestIdExhausted => "capability_request_id_exhausted",
        }
    }

    pub(crate) fn diagnostic(&self) -> String {
        self.to_string().chars().take(2_048).collect()
    }
}
