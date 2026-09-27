use super::super::protocol::ClientErrorCode;
use super::super::protocol_search_api::{
    self, SEARCH_API_PROTOCOL, SEARCH_API_VERSION, SEARCH_API_VERSION_2, SearchApiRequest,
};

pub(super) struct ParsedSearchRequest {
    pub(super) version: u16,
    pub(super) request: SearchApiRequest,
}

pub(super) struct InvalidSearchRequest {
    pub(super) version: u16,
    pub(super) message: String,
    pub(super) error_code: ClientErrorCode,
}

pub(super) fn parse(value: serde_json::Value) -> Result<ParsedSearchRequest, InvalidSearchRequest> {
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u16::try_from(version).ok())
        .ok_or_else(|| InvalidSearchRequest {
            version: SEARCH_API_VERSION_2,
            message: "unsupported search API protocol or version".to_string(),
            error_code: ClientErrorCode::ProtocolVersionMismatch,
        })?;
    let matches_protocol =
        value.get("protocol").and_then(serde_json::Value::as_str) == Some(SEARCH_API_PROTOCOL);
    let matches_version = matches!(version, SEARCH_API_VERSION | SEARCH_API_VERSION_2);
    if !matches_protocol || !matches_version {
        return Err(InvalidSearchRequest {
            version: SEARCH_API_VERSION_2,
            message: "unsupported search API protocol or version".to_string(),
            error_code: ClientErrorCode::ProtocolVersionMismatch,
        });
    }
    let request = serde_json::from_value::<SearchApiRequest>(value).map_err(|error| {
        InvalidSearchRequest {
            version,
            message: format!("invalid search API request: {error}"),
            error_code: ClientErrorCode::InvalidInput,
        }
    })?;
    if version == SEARCH_API_VERSION
        && matches!(
            &request.operation,
            super::super::protocol_search_api::SearchApiOperation::IndexingStatus
                | super::super::protocol_search_api::SearchApiOperation::InteractiveSearch { .. }
        )
    {
        return Err(InvalidSearchRequest {
            version,
            message: "this search API operation requires version 2".to_string(),
            error_code: ClientErrorCode::ProtocolVersionMismatch,
        });
    }
    if let Err(error) = protocol_search_api::validate_operation(&request.operation) {
        return Err(InvalidSearchRequest {
            version,
            message: error.message,
            error_code: error.code,
        });
    }
    Ok(ParsedSearchRequest { version, request })
}
