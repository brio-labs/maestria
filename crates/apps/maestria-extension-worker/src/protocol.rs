use maestria_extensions::{
    CapabilityFailure, CapabilityRequest, CapabilityResponse, CapabilitySuccess, FormValues,
    HostMessage, OpenRequestTarget, StorageOperation,
};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::error::WorkerError;

pub(crate) struct Invocation {
    pub(crate) command_id: String,
    pub(crate) context_invocation: Value,
}

pub(crate) struct CapabilityCall {
    pub(crate) request_id: String,
    pub(crate) request: CapabilityRequest,
    pub(crate) response: oneshot::Sender<CapabilityResponse>,
}

pub(crate) enum BridgeEvent {
    Request(CapabilityCall),
    Failure(BridgeFailure),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum BridgeFailure {
    InvalidRequest,
    RequestLimit,
    RequestIdExhausted,
    ResponseEncoding,
}

#[derive(serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum InvocationValue {
    Command {
        input: FormValues,
    },
    Action {
        action_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        values: FormValues,
    },
}

pub(crate) fn invocation_from_host(message: HostMessage) -> Result<Invocation, WorkerError> {
    match message {
        HostMessage::CommandInvoke {
            command_id, input, ..
        } => Ok(Invocation {
            command_id,
            context_invocation: serde_json::to_value(InvocationValue::Command { input })?,
        }),
        HostMessage::ActionInvoke {
            command_id,
            action_id,
            item_id,
            values,
            ..
        } => Ok(Invocation {
            command_id,
            context_invocation: serde_json::to_value(InvocationValue::Action {
                action_id,
                item_id,
                values,
            })?,
        }),
        _ => Err(WorkerError::UnexpectedHostMessage),
    }
}

pub(crate) fn parse_capability_request(
    encoded: &str,
) -> Result<CapabilityRequest, maestria_extensions::ProtocolError> {
    let raw: Value = serde_json::from_str(encoded)?;
    validate_capability_request_shape(&raw)?;
    let request: CapabilityRequest = serde_json::from_value(raw)?;
    maestria_extensions::validate_capability_request(&request)?;
    Ok(request)
}

fn validate_capability_request_shape(
    raw: &Value,
) -> Result<(), maestria_extensions::ProtocolError> {
    let object = raw
        .as_object()
        .ok_or(maestria_extensions::ProtocolError::Invalid(
            "capability request",
        ))?;
    match object.get("capability").and_then(Value::as_str) {
        Some("http") => {
            if object.get("body").is_some_and(|body| !body.is_string()) {
                return Err(maestria_extensions::ProtocolError::Invalid(
                    "HTTP request body",
                ));
            }
        }
        Some("storage") => {
            let operation = object.get("operation").and_then(Value::as_str);
            let value = object.get("value");
            match operation {
                Some("set") if value.and_then(Value::as_str).is_some() => {}
                Some("get" | "delete") if value.is_none() => {}
                _ => {
                    return Err(maestria_extensions::ProtocolError::Invalid(
                        "storage request shape",
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn capability_response_json(
    response: &CapabilityResponse,
) -> Result<String, maestria_extensions::ProtocolError> {
    let mut encoded = serde_json::to_value(response)?;
    match response {
        CapabilityResponse::Success(CapabilitySuccess::FileSearch { results, .. }) => {
            omit_absent_search_snippets(&mut encoded, results)?;
        }
        CapabilityResponse::Success(CapabilitySuccess::Storage {
            operation: StorageOperation::Get,
            ..
        }) => {
            response_object_mut(&mut encoded)?.remove("completed");
        }
        CapabilityResponse::Success(CapabilitySuccess::Storage {
            operation: StorageOperation::Set | StorageOperation::Delete,
            ..
        }) => {
            response_object_mut(&mut encoded)?.remove("value");
        }
        _ => {}
    }
    Ok(serde_json::to_string(&encoded)?)
}

fn omit_absent_search_snippets(
    encoded: &mut Value,
    results: &[maestria_extensions::FileSearchResult],
) -> Result<(), maestria_extensions::ProtocolError> {
    let object = response_object_mut(encoded)?;
    let Some(encoded_results) = object.get_mut("results").and_then(Value::as_array_mut) else {
        return Err(maestria_extensions::ProtocolError::Invalid(
            "file search response shape",
        ));
    };
    if encoded_results.len() != results.len() {
        return Err(maestria_extensions::ProtocolError::Invalid(
            "file search response shape",
        ));
    }
    for (encoded_result, result) in encoded_results.iter_mut().zip(results) {
        if result.snippet.is_none() {
            let Some(encoded_result) = encoded_result.as_object_mut() else {
                return Err(maestria_extensions::ProtocolError::Invalid(
                    "file search result shape",
                ));
            };
            encoded_result.remove("snippet");
        }
    }
    Ok(())
}

fn response_object_mut(
    encoded: &mut Value,
) -> Result<&mut serde_json::Map<String, Value>, maestria_extensions::ProtocolError> {
    encoded
        .as_object_mut()
        .ok_or(maestria_extensions::ProtocolError::Invalid(
            "capability response shape",
        ))
}

pub(crate) fn response_matches_request(
    request: &CapabilityRequest,
    response: &CapabilityResponse,
) -> bool {
    match response {
        CapabilityResponse::Success(success) => success_matches_request(request, success),
        CapabilityResponse::Failure(failure) => failure_matches_request(request, failure),
    }
}

fn failure_matches_request(request: &CapabilityRequest, failure: &CapabilityFailure) -> bool {
    let code_is_supported = matches!(
        failure.error.code.as_str(),
        "permission_denied"
            | "cancelled"
            | "not_found"
            | "invalid_request"
            | "unavailable"
            | "failed"
    );
    !failure.ok
        && failure.capability == request.kind()
        && code_is_supported
        && failure.error.message.chars().take(4_097).count() <= 4_096
}

fn success_matches_request(request: &CapabilityRequest, success: &CapabilitySuccess) -> bool {
    match (request, success) {
        (
            CapabilityRequest::FileSearch { .. },
            CapabilitySuccess::FileSearch { ok: true, results },
        ) => results.len() <= 100,
        (
            CapabilityRequest::UserFileRead { .. },
            CapabilitySuccess::UserFileRead { ok: true, text, .. },
        ) => text.chars().take(16_385).count() <= 16_384,
        (CapabilityRequest::Http { .. }, CapabilitySuccess::Http { ok: true, body, .. }) => {
            body.chars().take(16_385).count() <= 16_384
        }
        (
            CapabilityRequest::Storage {
                operation: StorageOperation::Get,
                ..
            },
            CapabilitySuccess::Storage {
                ok: true,
                operation: StorageOperation::Get,
                value,
                completed: None,
            },
        ) => value
            .as_ref()
            .is_none_or(|value| value.chars().take(16_385).count() <= 16_384),
        (
            CapabilityRequest::Storage {
                operation: StorageOperation::Set,
                ..
            },
            CapabilitySuccess::Storage {
                ok: true,
                operation: StorageOperation::Set,
                value: None,
                completed: Some(true),
            },
        ) => true,
        (
            CapabilityRequest::Storage {
                operation: StorageOperation::Delete,
                ..
            },
            CapabilitySuccess::Storage {
                ok: true,
                operation: StorageOperation::Delete,
                value: None,
                completed: Some(true),
            },
        ) => true,
        (
            CapabilityRequest::Notification { .. },
            CapabilitySuccess::Notification {
                ok: true,
                delivered: true,
            },
        ) => true,
        (
            CapabilityRequest::Open { target },
            CapabilitySuccess::Open {
                ok: true,
                opened: true,
            },
        ) => valid_open_target(target),
        (
            CapabilityRequest::Copy { .. },
            CapabilitySuccess::Copy {
                ok: true,
                copied: true,
            },
        ) => true,
        _ => false,
    }
}

fn valid_open_target(target: &OpenRequestTarget) -> bool {
    match target {
        OpenRequestTarget::Url { url } => {
            !url.is_empty() && url.chars().take(4_097).count() <= 4_096
        }
        OpenRequestTarget::SelectedFile { selection_id } => valid_key(selection_id),
    }
}

fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.chars().take(65).count() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
