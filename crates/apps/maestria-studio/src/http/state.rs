use std::sync::Arc;

use maestria_daemon::api::DaemonClient;

use crate::agent::AgentHost;

#[derive(Debug, Clone)]
pub(crate) struct StudioState {
    pub(crate) client: DaemonClient,
    pub(crate) agent: AgentHost,
    pub(crate) bearer: Arc<str>,
    pub(crate) origin: Arc<str>,
}
