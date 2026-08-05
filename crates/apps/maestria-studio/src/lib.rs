/// Responsibility map:
/// - `agent`: ACP v1 external-agent client and profile configuration.
/// - `http`: loopback HTTP routes, DTOs, authentication, and embedded assets.
/// - `server`: Studio lifecycle and ephemeral session handoff.
mod agent;
mod http;
mod server;

pub use agent::{AgentHost, AgentProfile};
pub use server::StudioServer;
