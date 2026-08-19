use std::fmt;

use maestria_ports::{PortError, ProviderTransport};
use serde::{Serialize, de::DeserializeOwned};

/// Posts a JSON request to the transport and deserializes the JSON response.
pub fn post_json<Req: Serialize, Resp: DeserializeOwned>(
    transport: &dyn ProviderTransport,
    request: &Req,
    context: &'static str,
) -> Result<Resp, PortError> {
    let body = serde_json::to_vec(request)
        .map_err(|error| PortError::internal("encode provider request", error.to_string()))?;
    let response_bytes = transport.post(body)?;
    serde_json::from_slice(&response_bytes).map_err(|error| {
        PortError::downstream(context, format!("decode provider response: {error}"))
    })
}

/// Posts a JSON request to a sibling path on the transport.
pub fn post_json_to<Req: Serialize, Resp: DeserializeOwned>(
    transport: &dyn ProviderTransport,
    path_suffix: &'static str,
    request: &Req,
    context: &'static str,
) -> Result<Resp, PortError> {
    let body = serde_json::to_vec(request)
        .map_err(|error| PortError::internal("encode provider batch request", error.to_string()))?;
    let response_bytes = transport.post_to(path_suffix, body)?;
    serde_json::from_slice(&response_bytes).map_err(|error| {
        PortError::downstream(context, format!("decode provider batch response: {error}"))
    })
}

/// Validates that a configured model name matches the model name in the identity fingerprint.
pub fn validate_model_identity(
    configured: &str,
    fingerprint: &str,
    provider_label: &'static str,
) -> Result<(), PortError> {
    if configured != fingerprint {
        return Err(PortError::invalid_input(
            "model identity mismatch",
            format!(
                "{provider_label} model '{configured}' does not match identity '{fingerprint}'"
            ),
        ));
    }
    Ok(())
}

/// Checks that two identity components are equal, returning an invalid-input error on mismatch.
pub fn require_identity_eq<T: PartialEq + fmt::Debug>(
    actual: &T,
    expected: &T,
    context: &'static str,
    message: &'static str,
) -> Result<(), PortError> {
    if actual != expected {
        return Err(PortError::invalid_input(context, message));
    }
    Ok(())
}
