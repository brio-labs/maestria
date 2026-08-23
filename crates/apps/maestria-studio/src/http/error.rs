use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use maestria_daemon::api::{ClientErrorCode, DaemonRequestError};
use serde::Serialize;

use crate::agent::AgentHostError;

pub const PROBLEM_PREFIX: &str = "urn:maestria:studio:problem:";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProblemCode {
    InvalidInput,
    Unauthorized,
    RequestTimeout,
    OriginDenied,
    SourceNotSelected,
    NotFound,
    MethodNotAllowed,
    SourceUnavailable,
    RevisionConflict,
    RequestTooLarge,
    NoEvidence,
    AgentUnconfigured,
    InvalidAgentOutput,
    AgentProtocol,
    AgentUnavailable,
    Internal,
}

#[derive(Debug)]
pub struct StudioError {
    pub(crate) code: ProblemCode,
    _source: Option<anyhow::Error>,
}

#[derive(Debug, Serialize)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    pub type_uri: String,
    pub title: &'static str,
    pub status: u16,
    pub detail: String,
}

impl ProblemCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid-input",
            Self::Unauthorized => "unauthorized",
            Self::RequestTimeout => "request-timeout",
            Self::OriginDenied => "origin-denied",
            Self::SourceNotSelected => "source-not-selected",
            Self::NotFound => "not-found",
            Self::MethodNotAllowed => "method-not-allowed",
            Self::SourceUnavailable => "source-unavailable",
            Self::RevisionConflict => "revision-conflict",
            Self::RequestTooLarge => "request-too-large",
            Self::NoEvidence => "no-evidence",
            Self::AgentUnconfigured => "agent-unconfigured",
            Self::InvalidAgentOutput => "invalid-agent-output",
            Self::AgentProtocol => "agent-protocol",
            Self::AgentUnavailable => "agent-unavailable",
            Self::Internal => "internal",
        }
    }

    pub const fn status(self) -> StatusCode {
        match self {
            Self::InvalidInput => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::RequestTimeout => StatusCode::REQUEST_TIMEOUT,
            Self::OriginDenied | Self::SourceNotSelected => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::SourceUnavailable | Self::RevisionConflict => StatusCode::CONFLICT,
            Self::RequestTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::NoEvidence => StatusCode::UNPROCESSABLE_ENTITY,
            Self::AgentUnconfigured
            | Self::InvalidAgentOutput
            | Self::AgentProtocol
            | Self::AgentUnavailable => StatusCode::BAD_GATEWAY,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::InvalidInput => "Invalid request",
            Self::Unauthorized => "Unauthorized",
            Self::RequestTimeout => "Request timeout",
            Self::OriginDenied => "Origin denied",
            Self::SourceNotSelected => "Source not selected",
            Self::NotFound => "Not found",
            Self::MethodNotAllowed => "Method not allowed",
            Self::SourceUnavailable => "Source unavailable",
            Self::RevisionConflict => "Revision conflict",
            Self::RequestTooLarge => "Request too large",
            Self::NoEvidence => "No evidence",
            Self::AgentUnconfigured => "Agent unconfigured",
            Self::InvalidAgentOutput => "Invalid agent output",
            Self::AgentProtocol => "Agent protocol error",
            Self::AgentUnavailable => "Agent unavailable",
            Self::Internal => "Internal error",
        }
    }

    pub const fn detail(self) -> &'static str {
        match self {
            Self::InvalidInput => "The request is invalid",
            Self::Unauthorized => "Studio session is unauthorized",
            Self::RequestTimeout => "The request body timed out",
            Self::OriginDenied => "Studio request origin is not allowed",
            Self::SourceNotSelected => "Evidence is not selected in this notebook",
            Self::NotFound => "The requested resource was not found",
            Self::MethodNotAllowed => "This method is not allowed for the resource",
            Self::SourceUnavailable => "A selected source is unavailable",
            Self::RevisionConflict => "The resource changed; reload and retry",
            Self::RequestTooLarge => "The request exceeds the Studio limit",
            Self::NoEvidence => "No grounded evidence was found",
            Self::AgentUnconfigured => "No Studio agent is configured",
            Self::InvalidAgentOutput => "The configured agent returned an invalid response",
            Self::AgentProtocol => "The configured agent could not complete the ACP exchange",
            Self::AgentUnavailable => "The configured Studio agent did not respond",
            Self::Internal => "Studio could not complete the request",
        }
    }
}

impl StudioError {
    pub fn new(code: ProblemCode) -> Self {
        Self {
            code,
            _source: None,
        }
    }
    pub fn with_source(code: ProblemCode, source: anyhow::Error) -> Self {
        Self {
            code,
            _source: Some(source),
        }
    }
}

impl From<DaemonRequestError> for StudioError {
    fn from(error: DaemonRequestError) -> Self {
        let code = match error.code {
            ClientErrorCode::Unauthorized => ProblemCode::Unauthorized,
            ClientErrorCode::InvalidInput => ProblemCode::InvalidInput,
            ClientErrorCode::NotFound => ProblemCode::NotFound,
            ClientErrorCode::SourceUnavailable => ProblemCode::SourceUnavailable,
            ClientErrorCode::SourceNotSelected => ProblemCode::SourceNotSelected,
            ClientErrorCode::RevisionConflict => ProblemCode::RevisionConflict,
            ClientErrorCode::NoEvidence => ProblemCode::NoEvidence,
            ClientErrorCode::RequestTooLarge => ProblemCode::RequestTooLarge,
            ClientErrorCode::DaemonUnavailable => ProblemCode::SourceUnavailable,
            ClientErrorCode::Internal => ProblemCode::Internal,
        };
        Self::with_source(code, anyhow::Error::new(error))
    }
}

impl From<AgentHostError> for StudioError {
    fn from(error: AgentHostError) -> Self {
        let code = match error {
            AgentHostError::Unconfigured => ProblemCode::AgentUnconfigured,
            AgentHostError::Timeout => ProblemCode::AgentUnavailable,
            AgentHostError::OutputTooLarge => ProblemCode::InvalidAgentOutput,
            AgentHostError::Protocol(source) => {
                return Self::with_source(ProblemCode::AgentProtocol, source);
            }
        };
        Self::new(code)
    }
}

impl IntoResponse for StudioError {
    fn into_response(self) -> Response {
        // Surface the underlying cause (e.g. the daemon's error message)
        // when one exists; the static per-code detail is only the fallback.
        let detail = match &self._source {
            Some(source) => source.to_string(),
            None => self.code.detail().to_string(),
        };
        let details = ProblemDetails {
            type_uri: format!("{PROBLEM_PREFIX}{}", self.code.as_str()),
            title: self.code.title(),
            status: self.code.status().as_u16(),
            detail,
        };
        let mut response = (self.code.status(), Json(details)).into_response();
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

#[cfg(test)]
mod tests {
    use super::{PROBLEM_PREFIX, ProblemCode};
    #[test]
    fn problem_mapping_is_stable() {
        assert_eq!(ProblemCode::RevisionConflict.status().as_u16(), 409);
        assert_eq!(
            format!("{PROBLEM_PREFIX}{}", ProblemCode::InvalidInput.as_str()),
            "urn:maestria:studio:problem:invalid-input"
        );
    }

    #[test]
    fn problem_table_covers_every_wire_category() {
        let cases = [
            (ProblemCode::InvalidInput, 400, "invalid-input"),
            (ProblemCode::Unauthorized, 401, "unauthorized"),
            (ProblemCode::RequestTimeout, 408, "request-timeout"),
            (ProblemCode::OriginDenied, 403, "origin-denied"),
            (ProblemCode::SourceNotSelected, 403, "source-not-selected"),
            (ProblemCode::NotFound, 404, "not-found"),
            (ProblemCode::MethodNotAllowed, 405, "method-not-allowed"),
            (ProblemCode::SourceUnavailable, 409, "source-unavailable"),
            (ProblemCode::RevisionConflict, 409, "revision-conflict"),
            (ProblemCode::RequestTooLarge, 413, "request-too-large"),
            (ProblemCode::NoEvidence, 422, "no-evidence"),
            (ProblemCode::AgentUnconfigured, 502, "agent-unconfigured"),
            (ProblemCode::InvalidAgentOutput, 502, "invalid-agent-output"),
            (ProblemCode::AgentProtocol, 502, "agent-protocol"),
            (ProblemCode::AgentUnavailable, 502, "agent-unavailable"),
            (ProblemCode::Internal, 500, "internal"),
        ];
        for (code, status, name) in cases {
            assert_eq!(code.status().as_u16(), status);
            assert_eq!(code.as_str(), name);
            assert!(!code.title().is_empty());
            assert!(!code.detail().is_empty());
        }
    }
}
