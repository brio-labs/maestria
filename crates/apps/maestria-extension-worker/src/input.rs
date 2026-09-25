use std::{
    io::{self, BufRead},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use maestria_extensions::{
    CapabilityResponse, FormValues, HostMessage, MAX_CAPABILITY_RESPONSE_BYTES,
    MAX_JSON_LINE_BYTES, PROTOCOL_VERSION,
};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug)]
pub(crate) enum InputEvent {
    Message(HostMessage),
    Failure(InputError),
    Closed,
}

#[derive(Debug, Error)]
pub(crate) enum InputError {
    #[error("host message line exceeds the protocol byte limit")]
    Oversized,
    #[error("host message has invalid JSON-line framing")]
    Framing,
    #[error("host message JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("host message protocol version {0} is unsupported")]
    Version(u32),
    #[error("host message violates the protocol: {0}")]
    Invalid(&'static str),
    #[error("host input failed: {0}")]
    Io(#[from] io::Error),
}

pub(crate) fn start_reader(
    interrupted: Arc<AtomicBool>,
) -> Result<mpsc::Receiver<InputEvent>, io::Error> {
    let (sender, receiver) = mpsc::channel(32);
    thread::Builder::new()
        .name("sillage-extension-stdin".to_owned())
        .spawn(move || read_messages(sender, interrupted))?;
    Ok(receiver)
}

fn read_messages(sender: mpsc::Sender<InputEvent>, interrupted: Arc<AtomicBool>) {
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    loop {
        let line = match read_bounded_line(&mut stdin) {
            Ok(Some(line)) => line,
            Ok(None) => {
                interrupted.store(true, Ordering::Release);
                if sender.blocking_send(InputEvent::Closed).is_err() {
                    return;
                }
                return;
            }
            Err(error) => {
                interrupted.store(true, Ordering::Release);
                if sender.blocking_send(InputEvent::Failure(error)).is_err() {
                    return;
                }
                return;
            }
        };
        let message = match parse_host_line(&line) {
            Ok(message) => message,
            Err(error) => {
                interrupted.store(true, Ordering::Release);
                if sender.blocking_send(InputEvent::Failure(error)).is_err() {
                    return;
                }
                return;
            }
        };
        if matches!(&message, HostMessage::CommandCancel { .. }) {
            interrupted.store(true, Ordering::Release);
        }
        if sender.blocking_send(InputEvent::Message(message)).is_err() {
            return;
        }
    }
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>, InputError> {
    let mut line = Vec::with_capacity(1_024);
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }

        let delimiter = available.iter().position(|byte| *byte == b'\n');
        let content_length = match delimiter {
            Some(position) => position,
            None => available.len(),
        };
        if line.len().saturating_add(content_length) > MAX_JSON_LINE_BYTES {
            return Err(InputError::Oversized);
        }
        line.extend_from_slice(&available[..content_length]);
        let consumed = content_length + usize::from(delimiter.is_some());
        reader.consume(consumed);
        if delimiter.is_some() {
            return Ok(Some(line));
        }
    }
}

fn parse_host_line(line: &[u8]) -> Result<HostMessage, InputError> {
    if line.len() > MAX_JSON_LINE_BYTES {
        return Err(InputError::Oversized);
    }
    if line.contains(&b'\n') || line.contains(&b'\r') {
        return Err(InputError::Framing);
    }
    let raw: Value = serde_json::from_slice(line)?;
    validate_optional_wire_fields(&raw)?;
    let message: HostMessage = serde_json::from_value(raw)?;
    let version = protocol_version(&message);
    if version != PROTOCOL_VERSION {
        return Err(InputError::Version(version));
    }
    validate_message(&message)?;
    Ok(message)
}

fn validate_optional_wire_fields(raw: &Value) -> Result<(), InputError> {
    let object = raw
        .as_object()
        .ok_or(InputError::Invalid("message object"))?;
    match object.get("kind").and_then(Value::as_str) {
        Some("action.invoke") if object.get("itemId").is_some_and(Value::is_null) => {
            Err(InputError::Invalid("action item ID"))
        }
        Some("capability.response") => validate_capability_response_fields(object),
        _ => Ok(()),
    }
}

fn validate_capability_response_fields(
    message: &serde_json::Map<String, Value>,
) -> Result<(), InputError> {
    let Some(response) = message.get("response").and_then(Value::as_object) else {
        return Err(InputError::Invalid("capability response object"));
    };
    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        return Ok(());
    }
    match response.get("capability").and_then(Value::as_str) {
        Some("fileSearch") => validate_file_search_fields(response),
        Some("storage") => validate_storage_response_fields(response),
        _ => Ok(()),
    }
}

fn validate_file_search_fields(
    response: &serde_json::Map<String, Value>,
) -> Result<(), InputError> {
    let Some(results) = response.get("results").and_then(Value::as_array) else {
        return Err(InputError::Invalid("file search results"));
    };
    for result in results {
        let Some(result) = result.as_object() else {
            return Err(InputError::Invalid("file search result"));
        };
        if result
            .get("snippet")
            .is_some_and(|snippet| !snippet.is_string())
        {
            return Err(InputError::Invalid("file search snippet"));
        }
    }
    Ok(())
}

fn validate_storage_response_fields(
    response: &serde_json::Map<String, Value>,
) -> Result<(), InputError> {
    match response.get("operation").and_then(Value::as_str) {
        Some("get") => {
            let Some(value) = response.get("value") else {
                return Err(InputError::Invalid("storage get response value"));
            };
            if !value.is_null() && !value.is_string() || response.contains_key("completed") {
                return Err(InputError::Invalid("storage get response shape"));
            }
        }
        Some("set" | "delete")
            if response.get("completed") == Some(&Value::Bool(true))
                && !response.contains_key("value") => {}
        Some("set" | "delete") => {
            return Err(InputError::Invalid("storage mutation response shape"));
        }
        _ => {}
    }
    Ok(())
}

fn protocol_version(message: &HostMessage) -> u32 {
    match message {
        HostMessage::CommandInvoke {
            protocol_version, ..
        }
        | HostMessage::ActionInvoke {
            protocol_version, ..
        }
        | HostMessage::CommandCancel {
            protocol_version, ..
        }
        | HostMessage::CapabilityResponse {
            protocol_version, ..
        } => *protocol_version,
    }
}

fn validate_message(message: &HostMessage) -> Result<(), InputError> {
    match message {
        HostMessage::CommandInvoke {
            command_id, input, ..
        } => {
            validate_key(command_id, "command ID")?;
            validate_form_values(input)?;
        }
        HostMessage::ActionInvoke {
            command_id,
            action_id,
            item_id,
            values,
            ..
        } => {
            validate_key(command_id, "command ID")?;
            validate_key(action_id, "action ID")?;
            if let Some(item_id) = item_id {
                validate_key(item_id, "item ID")?;
            }
            validate_form_values(values)?;
        }
        HostMessage::CommandCancel { command_id, .. } => {
            validate_key(command_id, "command ID")?;
        }
        HostMessage::CapabilityResponse {
            request_id,
            response,
            ..
        } => {
            validate_key(request_id, "request ID")?;
            validate_response_size(response)?;
        }
    }
    Ok(())
}

fn validate_form_values(values: &FormValues) -> Result<(), InputError> {
    if values.len() > 32 {
        return Err(InputError::Invalid("form value field count"));
    }
    for (key, value) in values {
        validate_key(key, "form value key")?;
        if let maestria_extensions::FormValue::Text(text) = value
            && text.chars().take(4_097).count() > 4_096
        {
            return Err(InputError::Invalid("form value text length"));
        }
    }
    Ok(())
}

fn validate_key(value: &str, field: &'static str) -> Result<(), InputError> {
    if value.is_empty()
        || value.chars().take(65).count() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(InputError::Invalid(field));
    }
    Ok(())
}

fn validate_response_size(response: &CapabilityResponse) -> Result<(), InputError> {
    let encoded = serde_json::to_vec(response)?;
    if encoded.len() > MAX_CAPABILITY_RESPONSE_BYTES {
        return Err(InputError::Invalid("capability response byte length"));
    }
    Ok(())
}
