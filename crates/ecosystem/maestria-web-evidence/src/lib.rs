#![forbid(unsafe_code)]

//! Synchronous web evidence adapter backed by `ureq`.

use maestria_ports::{PortError, WebFetcher, WebSnapshotData};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct UreqWebFetcher {
    agent: ureq::Agent,
}

impl Default for UreqWebFetcher {
    fn default() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(15))
                .build(),
        }
    }
}

impl UreqWebFetcher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl WebFetcher for UreqWebFetcher {
    fn fetch(&self, url: &str) -> Result<WebSnapshotData, PortError> {
        let response = match self.agent.get(url).call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(404, _)) => return Err(PortError::NotFound),
            Err(e) => return Err(downstream_error(e)),
        };
        let html = response.into_string().map_err(downstream_error)?;

        Ok(WebSnapshotData {
            url: url.to_string(),
            html,
        })
    }
}

fn downstream_error(error: impl std::fmt::Display) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
    }
}
