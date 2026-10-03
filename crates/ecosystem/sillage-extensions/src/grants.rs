use thiserror::Error;
use url::Url;

use crate::{
    CapabilityRequest, OpenRequestTarget, OpenTarget, Permission, ProtocolError,
    validate_capability_request,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationOrigin {
    Command,
    ExplicitAction,
}

#[derive(Debug, Error)]
pub enum GrantError {
    #[error("invalid extension request: {0}")]
    Invalid(#[from] ProtocolError),
    #[error("extension has no active {0} grant")]
    Missing(&'static str),
    #[error("requested {0} exceeds the granted scope")]
    Scope(&'static str),
    #[error("{0} requires an explicit user action")]
    UserAction(&'static str),
}

/// Policy only. The broker must load the active extension's persisted grant
/// before every call, then execute the returned request through a typed adapter.
/// An installed manifest alone never grants a capability.
pub fn authorize(
    active_grants: &[Permission],
    request: &CapabilityRequest,
    origin: InvocationOrigin,
) -> Result<(), GrantError> {
    validate_capability_request(request)?;
    let permission = active_grants
        .iter()
        .find(|permission| permission.kind() == request.kind())
        .ok_or(GrantError::Missing(request.kind()))?;
    match (permission, request) {
        (Permission::FileSearch { max_results }, CapabilityRequest::FileSearch { limit, .. }) => {
            if *limit > max_results.map_or(100, usize::from) {
                return Err(GrantError::Scope("fileSearch.maxResults"));
            }
        }
        (Permission::Http { origins }, CapabilityRequest::Http { url, .. }) => {
            let url = Url::parse(url).map_err(|_| GrantError::Scope("http.origin"))?;
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || !origins.iter().any(|granted| {
                    Url::parse(granted).is_ok_and(|origin| origin.origin() == url.origin())
                })
            {
                return Err(GrantError::Scope("http.origin"));
            }
        }
        (Permission::Open { targets }, CapabilityRequest::Open { target }) => {
            require_user_action(origin, "open")?;
            let granted = match target {
                OpenRequestTarget::Url { url } => {
                    let parsed = Url::parse(url).map_err(|_| GrantError::Scope("open.url"))?;
                    parsed.scheme() == "https"
                        && parsed.username().is_empty()
                        && parsed.password().is_none()
                        && targets.contains(&OpenTarget::Url)
                }
                OpenRequestTarget::SelectedFile { .. } => {
                    targets.contains(&OpenTarget::SelectedFile)
                }
            };
            if !granted {
                return Err(GrantError::Scope("open.targets"));
            }
        }
        (Permission::Copy { .. }, CapabilityRequest::Copy { .. }) => {
            require_user_action(origin, "copy")?;
        }
        (Permission::UserFileRead, CapabilityRequest::UserFileRead { .. })
        | (Permission::Storage { .. }, CapabilityRequest::Storage { .. })
        | (Permission::Notification, CapabilityRequest::Notification { .. }) => {}
        _ => return Err(GrantError::Missing(request.kind())),
    }
    Ok(())
}

fn require_user_action(origin: InvocationOrigin, action: &'static str) -> Result<(), GrantError> {
    if origin != InvocationOrigin::ExplicitAction {
        return Err(GrantError::UserAction(action));
    }
    Ok(())
}
