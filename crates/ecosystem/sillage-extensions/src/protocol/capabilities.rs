use serde::{Deserialize, Serialize};

use super::{
    ProtocolError,
    views::{bounded, valid_key},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum OpenRequestTarget {
    Url {
        url: String,
    },
    SelectedFile {
        #[serde(rename = "selectionId")]
        selection_id: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HttpMethod {
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "POST")]
    Post,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageOperation {
    Get,
    Set,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "capability", rename_all = "camelCase", deny_unknown_fields)]
pub enum CapabilityRequest {
    FileSearch {
        query: String,
        limit: usize,
    },
    UserFileRead {
        #[serde(rename = "selectionId")]
        selection_id: String,
        #[serde(rename = "maxBytes")]
        max_bytes: usize,
    },
    Http {
        url: String,
        method: HttpMethod,
        #[serde(default)]
        body: Option<String>,
    },
    Storage {
        operation: StorageOperation,
        key: String,
        #[serde(default)]
        value: Option<String>,
    },
    Notification {
        title: String,
        message: String,
    },
    Open {
        target: OpenRequestTarget,
    },
    Copy {
        text: String,
    },
}

impl CapabilityRequest {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::FileSearch { .. } => "fileSearch",
            Self::UserFileRead { .. } => "userFileRead",
            Self::Http { .. } => "http",
            Self::Storage { .. } => "storage",
            Self::Notification { .. } => "notification",
            Self::Open { .. } => "open",
            Self::Copy { .. } => "copy",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileSearchResult {
    pub file_id: String,
    pub title: String,
    #[serde(default)]
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "capability", rename_all = "camelCase", deny_unknown_fields)]
pub enum CapabilitySuccess {
    FileSearch {
        ok: bool,
        results: Vec<FileSearchResult>,
    },
    UserFileRead {
        ok: bool,
        text: String,
        truncated: bool,
    },
    Http {
        ok: bool,
        status: u16,
        body: String,
        truncated: bool,
    },
    Storage {
        ok: bool,
        operation: StorageOperation,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        completed: Option<bool>,
    },
    Notification {
        ok: bool,
        delivered: bool,
    },
    Open {
        ok: bool,
        opened: bool,
    },
    Copy {
        ok: bool,
        copied: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityFailure {
    pub ok: bool,
    pub capability: String,
    pub error: CapabilityError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CapabilityResponse {
    Success(CapabilitySuccess),
    Failure(CapabilityFailure),
}

pub fn validate_capability_request(request: &CapabilityRequest) -> Result<(), ProtocolError> {
    match request {
        CapabilityRequest::FileSearch { query, limit }
            if query.trim().is_empty() || !bounded(query, 4096) || !(1..=100).contains(limit) =>
        {
            Err(ProtocolError::Invalid("file search bounds"))
        }
        CapabilityRequest::UserFileRead {
            selection_id,
            max_bytes,
        } if !valid_key(selection_id) || !(1..=16_384).contains(max_bytes) => {
            Err(ProtocolError::Invalid("selected file read bounds"))
        }
        CapabilityRequest::Http { url, body, .. }
            if !bounded(url, 4096) || body.as_ref().is_some_and(|body| !bounded(body, 16_384)) =>
        {
            Err(ProtocolError::Invalid("HTTP request bounds"))
        }
        CapabilityRequest::Storage { key, value, .. }
            if !valid_key(key) || value.as_ref().is_some_and(|value| !bounded(value, 16_384)) =>
        {
            Err(ProtocolError::Invalid("storage request bounds"))
        }
        CapabilityRequest::Storage {
            operation: StorageOperation::Set,
            value: None,
            ..
        }
        | CapabilityRequest::Storage {
            operation: StorageOperation::Get | StorageOperation::Delete,
            value: Some(_),
            ..
        } => Err(ProtocolError::Invalid("storage operation and value")),
        CapabilityRequest::Notification { title, message }
            if !bounded(title, 120) || !bounded(message, 4096) =>
        {
            Err(ProtocolError::Invalid("notification text"))
        }
        CapabilityRequest::Open {
            target: OpenRequestTarget::Url { url },
        } if !bounded(url, 4096) => Err(ProtocolError::Invalid("open URL")),
        CapabilityRequest::Open {
            target: OpenRequestTarget::SelectedFile { selection_id },
        } if !valid_key(selection_id) => Err(ProtocolError::Invalid("selected file ID")),
        CapabilityRequest::Copy { text } if !bounded(text, 16_384) => {
            Err(ProtocolError::Invalid("clipboard text"))
        }
        _ => Ok(()),
    }
}

pub(super) fn validate_capability_response(
    response: &CapabilityResponse,
) -> Result<(), ProtocolError> {
    match response {
        CapabilityResponse::Success(success) => validate_success(success),
        CapabilityResponse::Failure(failure) => {
            let known_capability = matches!(
                failure.capability.as_str(),
                "fileSearch"
                    | "userFileRead"
                    | "http"
                    | "storage"
                    | "notification"
                    | "open"
                    | "copy"
            );
            let known_code = matches!(
                failure.error.code.as_str(),
                "permission_denied"
                    | "cancelled"
                    | "not_found"
                    | "invalid_request"
                    | "unavailable"
                    | "failed"
            );
            if failure.ok
                || !known_capability
                || !known_code
                || !bounded(&failure.error.message, 4096)
            {
                return Err(ProtocolError::Invalid("capability failure"));
            }
            Ok(())
        }
    }
}

fn validate_success(success: &CapabilitySuccess) -> Result<(), ProtocolError> {
    let valid = match success {
        CapabilitySuccess::FileSearch { ok, results } => {
            *ok && results.len() <= 100
                && results.iter().all(|result| {
                    valid_key(&result.file_id)
                        && bounded(&result.title, 120)
                        && result
                            .snippet
                            .as_deref()
                            .is_none_or(|text| bounded(text, 4096))
                })
        }
        CapabilitySuccess::UserFileRead { ok, text, .. } => *ok && bounded(text, 16_384),
        CapabilitySuccess::Http {
            ok, status, body, ..
        } => *ok && (100..=599).contains(status) && bounded(body, 16_384),
        CapabilitySuccess::Storage {
            ok,
            operation,
            value,
            completed,
        } => {
            *ok && match operation {
                StorageOperation::Get => {
                    completed.is_none() && value.as_deref().is_none_or(|text| bounded(text, 16_384))
                }
                StorageOperation::Set | StorageOperation::Delete => {
                    value.is_none() && *completed == Some(true)
                }
            }
        }
        CapabilitySuccess::Notification { ok, delivered } => *ok && *delivered,
        CapabilitySuccess::Open { ok, opened } => *ok && *opened,
        CapabilitySuccess::Copy { ok, copied } => *ok && *copied,
    };
    if !valid {
        return Err(ProtocolError::Invalid("capability success"));
    }
    Ok(())
}
