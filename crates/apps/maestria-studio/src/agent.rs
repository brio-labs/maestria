use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_client_protocol::{
    AcpAgent, Client, Error as AgentProtocolError,
    schema::{
        ProtocolVersion,
        v1::{
            ClientCapabilities, ContentBlock, ContentChunk, Implementation, InitializeRequest,
            SessionNotification, SessionUpdate, StopReason,
        },
    },
    util::MatchDispatch,
};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

const DEFAULT_AGENT_TIMEOUT_SECS: u64 = 120;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 65_536;
const MIN_MAX_OUTPUT_BYTES: usize = 4_096;
const MAX_MAX_OUTPUT_BYTES: usize = 262_144;

#[derive(Debug, Clone, Serialize)]
pub struct AgentProfile {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing)]
    pub command: String,
    #[serde(skip_serializing)]
    pub args: Vec<String>,
    pub status: String,
    pub config_options: Vec<String>,
    #[serde(skip_serializing)]
    pub timeout_secs: u64,
    #[serde(skip_serializing)]
    pub max_output_bytes: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentConfig {
    default_agent: String,
    agents: Vec<ConfiguredAgent>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfiguredAgent {
    id: String,
    label: String,
    command: String,
    args: Vec<String>,
    timeout_secs: u64,
    max_output_bytes: usize,
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            id: "omp".to_owned(),
            label: "Oh My Pi".to_owned(),
            command: "omp".to_owned(),
            args: vec![
                "--no-tools".to_owned(),
                "--no-session".to_owned(),
                "acp".to_owned(),
            ],
            status: if command_available("omp") {
                "ready".to_owned()
            } else {
                "agent_unconfigured".to_owned()
            },
            config_options: Vec::new(),
            timeout_secs: DEFAULT_AGENT_TIMEOUT_SECS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

impl AgentProfile {
    /// Load the selected profile from the instance TOML file. The parser is intentionally strict:
    /// unknown keys, malformed TOML, duplicate IDs, and invalid bounds fail startup.
    pub fn from_config(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("read Studio agent config {}", path.display()))?;
        let config: AgentConfig = toml::from_str(&contents)
            .with_context(|| format!("parse Studio agent config {}", path.display()))?;
        if config.default_agent.trim().is_empty() {
            return Err(anyhow!("Studio agent default_agent must not be empty"));
        }
        if config.agents.is_empty() {
            return Err(anyhow!(
                "Studio agent config must define at least one agent"
            ));
        }
        let mut selected = None;
        let mut ids = std::collections::BTreeSet::new();
        for configured in config.agents {
            if configured.id.trim().is_empty()
                || configured.label.trim().is_empty()
                || configured.command.trim().is_empty()
                || configured.args.is_empty()
                || configured.args.iter().any(|arg| arg.trim().is_empty())
            {
                return Err(anyhow!(
                    "Studio agent id, label, command, and args must not be empty"
                ));
            }
            if !ids.insert(configured.id.clone()) {
                return Err(anyhow!("duplicate Studio agent id {}", configured.id));
            }
            if !(1..=600).contains(&configured.timeout_secs) {
                return Err(anyhow!(
                    "Studio agent {} timeout_secs must be 1..=600",
                    configured.id
                ));
            }
            if !(MIN_MAX_OUTPUT_BYTES..=MAX_MAX_OUTPUT_BYTES).contains(&configured.max_output_bytes)
            {
                return Err(anyhow!(
                    "Studio agent {} max_output_bytes must be {MIN_MAX_OUTPUT_BYTES}..={MAX_MAX_OUTPUT_BYTES}",
                    configured.id
                ));
            }
            if !command_available(&configured.command) {
                return Err(anyhow!(
                    "Studio agent {} command is not available on PATH",
                    configured.id
                ));
            }
            if configured.id == config.default_agent {
                selected = Some(configured);
            }
        }
        let selected = selected.ok_or_else(|| {
            anyhow!(
                "Studio agent default_agent {} does not name an existing profile",
                config.default_agent
            )
        })?;
        Ok(Self {
            id: selected.id,
            label: selected.label,
            command: selected.command,
            args: selected.args,
            status: "ready".to_owned(),
            config_options: Vec::new(),
            timeout_secs: selected.timeout_secs,
            max_output_bytes: selected.max_output_bytes,
        })
    }
}

fn command_available(command: &str) -> bool {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return path.is_file();
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(command))
        .any(|candidate| candidate.is_file())
}

#[derive(Debug, Serialize)]
struct AcpServerConfig<'a> {
    #[serde(rename = "type")]
    server_type: &'static str,
    name: &'a str,
    command: &'a str,
    args: &'a [String],
    env: [&'static str; 0],
}

#[derive(Debug, Clone)]
pub struct AgentHost {
    profile: AgentProfile,
    workdir: PathBuf,
}
#[derive(Debug)]
pub enum AgentHostError {
    Unconfigured,
    Timeout,
    OutputTooLarge,
    Protocol(anyhow::Error),
}

impl std::fmt::Display for AgentHostError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unconfigured => formatter.write_str("agent is unconfigured"),
            Self::Timeout => formatter.write_str("ACP agent timed out"),
            Self::OutputTooLarge => formatter.write_str("ACP agent output exceeds Studio limit"),
            Self::Protocol(error) => write!(formatter, "ACP v1 session failed: {error}"),
        }
    }
}
impl std::error::Error for AgentHostError {}
fn acp_agent(profile: &AgentProfile) -> Result<AcpAgent, AgentHostError> {
    let server = AcpServerConfig {
        server_type: "stdio",
        name: &profile.id,
        command: &profile.command,
        args: &profile.args,
        env: [],
    };
    let command = serde_json::to_string(&server)
        .map_err(|error| AgentHostError::Protocol(anyhow::Error::new(error)))?;
    command.parse::<AcpAgent>().map_err(|error| {
        AgentHostError::Protocol(anyhow::Error::msg(format!(
            "parse ACP agent configuration: {error}"
        )))
    })
}

impl AgentHost {
    pub fn new(profile: AgentProfile) -> Self {
        Self {
            profile,
            workdir: match std::env::current_dir() {
                Ok(path) => path,
                Err(_) => PathBuf::from("."),
            },
        }
    }

    pub fn new_with_workdir(profile: AgentProfile, workdir: PathBuf) -> Self {
        Self { profile, workdir }
    }

    pub fn profile(&self) -> AgentProfile {
        self.profile.clone()
    }

    /// Runs one isolated ACP v1 session. Studio advertises no filesystem, terminal, or MCP
    /// capability by design; the SDK's default client dispatcher rejects unsolicited requests.
    ///
    /// # Cancellation
    ///
    /// Dropping the returned future cancels the bounded ACP operation and its child process.
    pub async fn ask(&self, prompt: String) -> std::result::Result<String, AgentHostError> {
        if self.profile.status == "agent_unconfigured" {
            return Err(AgentHostError::Unconfigured);
        }
        let agent = acp_agent(&self.profile)?;
        let workdir = self.workdir.clone();
        let max_output = self.profile.max_output_bytes;
        let result = timeout(
            Duration::from_secs(self.profile.timeout_secs),
            Client::connect_with(Client, agent, async move |connection| {
                let initialize = InitializeRequest::new(ProtocolVersion::V1)
                    .client_info(
                        Implementation::new("maestria-studio", env!("CARGO_PKG_VERSION"))
                            .title("Maestria Studio"),
                    )
                    .client_capabilities(ClientCapabilities::default());
                let initialized = connection.send_request(initialize).block_task().await?;
                if initialized.protocol_version != ProtocolVersion::V1 {
                    return Err(agent_client_protocol::Error::internal_error()
                        .data("ACP agent did not negotiate protocol v1"));
                }
                connection
                    .build_session(&workdir)
                    .block_task()
                    .run_until(async |mut session| {
                        session.send_prompt(prompt.clone())?;
                        let mut answer = String::new();
                        loop {
                            match session.read_update().await? {
                                agent_client_protocol::SessionMessage::StopReason(reason) => {
                                    if reason != StopReason::EndTurn {
                                        return Err(agent_client_protocol::Error::internal_error()
                                            .data(format!("ACP turn ended with {reason:?}")));
                                    }
                                    break;
                                }
                                agent_client_protocol::SessionMessage::SessionMessage(dispatch) => {
                                    MatchDispatch::new(dispatch)
                                        .if_notification(
                                            async |notification: SessionNotification| {
                                                if let SessionUpdate::AgentMessageChunk(
                                                    ContentChunk {
                                                        content: ContentBlock::Text(text),
                                                        ..
                                                    },
                                                ) = notification.update
                                                {
                                                    if answer.len().saturating_add(text.text.len())
                                                        > max_output
                                                    {
                                                        let error =
                                                            AgentProtocolError::internal_error()
                                                                .data("ACP agent output exceeds Studio limit");
                                                        return Err(error);
                                                    }
                                                    answer.push_str(&text.text);
                                                }
                                                Ok(())
                                            },
                                        )
                                        .await
                                        .otherwise_ignore()?;
                                }
                                _ => {
                                    return Err(agent_client_protocol::Error::internal_error()
                                        .data("ACP sent an unsupported session message"));
                                }
                            }
                        }
                        if answer.trim().is_empty() {
                            return Err(agent_client_protocol::Error::internal_error()
                                .data("ACP agent returned no answer"));
                        }
                        Ok(answer)
                    })
                    .await
            }),
        )
        .await
        .map_err(|_| AgentHostError::Timeout)?
        .map_err(|error| {
            let message = error.to_string();
            if message.contains("output exceeds Studio limit") {
                AgentHostError::OutputTooLarge
            } else {
                AgentHostError::Protocol(anyhow::Error::msg(message))
            }
        })?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentProfile, command_available};

    #[test]
    fn default_agent_has_safe_profile() {
        let profile = AgentProfile::default();
        assert_eq!(profile.args, vec!["--no-tools", "--no-session", "acp"]);
        assert_eq!(profile.timeout_secs, 120);
        assert_eq!(profile.max_output_bytes, 65_536);
    }

    #[test]
    fn malformed_and_unknown_config_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let path =
            std::env::temp_dir().join(format!("maestria-studio-agent-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "default_agent = \"x\"\n[[agents]]\nid=\"x\"\nlabel=\"X\"\ncommand=\"sh\"\nargs=[\"-c\",\"true\"]\ntimeout_secs=0\nmax_output_bytes=4096\n",
        )?;
        assert!(AgentProfile::from_config(&path).is_err());
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn path_command_resolution_distinguishes_missing_program() {
        assert!(command_available("sh"));
        assert!(!command_available("definitely-not-a-studio-agent"));
    }
    #[test]
    fn profile_serialization_does_not_expose_process_details()
    -> Result<(), Box<dyn std::error::Error>> {
        let profile = AgentProfile::default();
        let value = serde_json::to_value(profile)?;
        assert!(value.get("command").is_none());
        assert!(value.get("args").is_none());
        assert!(value.get("timeout_secs").is_none());
        Ok(())
    }
}
