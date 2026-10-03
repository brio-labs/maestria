use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    MAX_CAPABILITY_RESPONSE_BYTES, MAX_JSON_LINE_BYTES, PROTOCOL_VERSION,
    capabilities::{
        CapabilityRequest, CapabilityResponse, CapabilitySuccess, StorageOperation,
        validate_capability_request, validate_capability_response,
    },
    views::{FormValues, View, valid_key, validate_form_values, validate_view},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum HostMessage {
    #[serde(rename = "command.invoke")]
    CommandInvoke {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "commandId")]
        command_id: String,
        input: FormValues,
    },
    #[serde(rename = "action.invoke")]
    ActionInvoke {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "commandId")]
        command_id: String,
        #[serde(rename = "actionId")]
        action_id: String,
        #[serde(rename = "itemId", default)]
        item_id: Option<String>,
        values: FormValues,
    },
    #[serde(rename = "command.cancel")]
    CommandCancel {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "commandId")]
        command_id: String,
    },
    #[serde(rename = "capability.response")]
    CapabilityResponse {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "requestId")]
        request_id: String,
        response: CapabilityResponse,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum WorkerMessage {
    #[serde(rename = "view.update")]
    ViewUpdate {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "commandId")]
        command_id: String,
        view: View,
    },
    #[serde(rename = "capability.request")]
    CapabilityRequest {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "commandId")]
        command_id: String,
        #[serde(rename = "requestId")]
        request_id: String,
        request: CapabilityRequest,
    },
    #[serde(rename = "command.complete")]
    CommandComplete {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "commandId")]
        command_id: String,
    },
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("extension message exceeds {MAX_JSON_LINE_BYTES} bytes")]
    Oversized,
    #[error("extension capability response exceeds {MAX_CAPABILITY_RESPONSE_BYTES} bytes")]
    ResponseOversized,
    #[error("invalid extension JSON-line message: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported extension protocol version {0}; expected {PROTOCOL_VERSION}")]
    Version(u32),
    #[error("invalid extension {0}")]
    Invalid(&'static str),
}

impl WorkerMessage {
    pub fn parse_line(line: &[u8]) -> Result<Self, ProtocolError> {
        if line.len() > MAX_JSON_LINE_BYTES {
            return Err(ProtocolError::Oversized);
        }
        if line.contains(&b'\n') || line.contains(&b'\r') {
            return Err(ProtocolError::Invalid("line framing"));
        }
        let message: Self = serde_json::from_slice(line)?;
        let version = match &message {
            Self::ViewUpdate {
                protocol_version, ..
            }
            | Self::CapabilityRequest {
                protocol_version, ..
            }
            | Self::CommandComplete {
                protocol_version, ..
            } => *protocol_version,
        };
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::Version(version));
        }
        match &message {
            Self::ViewUpdate {
                command_id, view, ..
            } => {
                if !valid_key(command_id) {
                    return Err(ProtocolError::Invalid("command ID"));
                }
                validate_view(view)?;
            }
            Self::CapabilityRequest {
                command_id,
                request_id,
                request,
                ..
            } => {
                if !valid_key(command_id) || !valid_key(request_id) {
                    return Err(ProtocolError::Invalid("command or request ID"));
                }
                validate_capability_request(request)?;
            }
            Self::CommandComplete { command_id, .. } if !valid_key(command_id) => {
                return Err(ProtocolError::Invalid("command ID"));
            }
            _ => {}
        }
        Ok(message)
    }
}

impl HostMessage {
    /// Encode a host-reviewed message as one bounded UTF-8 JSON line. Callers
    /// write the returned bytes to the worker pipe without modifying framing.
    pub fn encode_line(&self) -> Result<Vec<u8>, ProtocolError> {
        let version = match self {
            Self::CommandInvoke {
                protocol_version, ..
            }
            | Self::ActionInvoke {
                protocol_version, ..
            }
            | Self::CommandCancel {
                protocol_version, ..
            }
            | Self::CapabilityResponse {
                protocol_version, ..
            } => *protocol_version,
        };
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::Version(version));
        }
        match self {
            Self::CommandInvoke {
                command_id, input, ..
            } => {
                if !valid_key(command_id) {
                    return Err(ProtocolError::Invalid("command ID"));
                }
                validate_form_values(input)?;
            }
            Self::ActionInvoke {
                command_id,
                action_id,
                item_id,
                values,
                ..
            } => {
                if !valid_key(command_id)
                    || !valid_key(action_id)
                    || item_id.as_deref().is_some_and(|id| !valid_key(id))
                {
                    return Err(ProtocolError::Invalid("command, action or item ID"));
                }
                validate_form_values(values)?;
            }
            Self::CommandCancel { command_id, .. } if !valid_key(command_id) => {
                return Err(ProtocolError::Invalid("command ID"));
            }
            Self::CapabilityResponse { request_id, .. } if !valid_key(request_id) => {
                return Err(ProtocolError::Invalid("request ID"));
            }
            _ => {}
        }
        let mut bytes = match self {
            Self::CapabilityResponse {
                request_id,
                response,
                ..
            } => encode_capability_response(request_id, response)?,
            _ => serde_json::to_vec(self)?,
        };
        if bytes.len() > MAX_JSON_LINE_BYTES {
            return Err(ProtocolError::Oversized);
        }
        bytes.push(b'\n');
        Ok(bytes)
    }
}

fn encode_capability_response(
    request_id: &str,
    response: &CapabilityResponse,
) -> Result<Vec<u8>, ProtocolError> {
    validate_capability_response(response)?;
    let mut response_bytes = match response {
        CapabilityResponse::Success(CapabilitySuccess::Storage { operation, .. }) => {
            let mut value = serde_json::to_value(response)?;
            let fields = value
                .as_object_mut()
                .ok_or(ProtocolError::Invalid("storage response"))?;
            match operation {
                StorageOperation::Get => {
                    fields.remove("completed");
                }
                StorageOperation::Set | StorageOperation::Delete => {
                    fields.remove("value");
                }
            }
            serde_json::to_vec(&value)?
        }
        _ => serde_json::to_vec(response)?,
    };
    if response_bytes.len() > MAX_CAPABILITY_RESPONSE_BYTES {
        return Err(ProtocolError::ResponseOversized);
    }
    let mut line = Vec::with_capacity(response_bytes.len() + request_id.len() + 90);
    line.extend_from_slice(
        b"{\"kind\":\"capability.response\",\"protocolVersion\":1,\"requestId\":",
    );
    serde_json::to_writer(&mut line, request_id)?;
    line.extend_from_slice(b",\"response\":");
    line.append(&mut response_bytes);
    line.push(b'}');
    Ok(line)
}
