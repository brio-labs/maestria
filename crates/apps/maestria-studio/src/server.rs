use std::{fs::File, io::Read, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_daemon::api::DaemonClient;
use tokio::{fs, net::TcpListener, sync::Semaphore};

use crate::{
    agent::{AgentHost, AgentProfile},
    http::{StudioState, serve},
};

pub struct StudioServer {
    address: SocketAddr,
    bearer: Arc<str>,
    task: tokio::task::JoinHandle<Result<()>>,
}

impl StudioServer {
    /// Binds an authenticated loopback server and connects to an already-running daemon.
    ///
    /// # Cancellation
    ///
    /// Dropping the returned future cancels startup and releases the listener.
    pub async fn start(instance_root: PathBuf, agent_config: Option<PathBuf>) -> Result<Self> {
        let layout = InstanceLayout::for_root(instance_root.clone());
        let client =
            DaemonClient::from_instance(&layout).context("connect Studio to Maestria daemon")?;
        let system_dir = instance_root.join("system");
        let config_path = match agent_config {
            Some(path) => path,
            None => system_dir.join("studio-agents.toml"),
        };
        let agent = AgentProfile::from_config(&config_path)?;
        let workdir = system_dir.join("studio-agent-workdir");
        fs::create_dir_all(&workdir)
            .await
            .with_context(|| format!("create Studio agent workdir {}", workdir.display()))?;
        let workdir = fs::canonicalize(&workdir)
            .await
            .with_context(|| format!("canonicalize Studio agent workdir {}", workdir.display()))?;
        let agent = AgentHost::new_with_workdir(agent, workdir);
        let bearer: Arc<str> = Arc::from(ephemeral_bearer()?);
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("bind Studio loopback")?;
        let address = listener
            .local_addr()
            .context("read Studio loopback address")?;
        let state = StudioState {
            client,
            agent,
            bearer: bearer.clone(),
            origin: Arc::from(format!("http://{address}")),
            request_slots: Arc::new(Semaphore::new(32)),
        };
        let task = tokio::spawn(async move { serve(listener, state).await });
        Ok(Self {
            address,
            bearer,
            task,
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn url(&self) -> String {
        format!("http://{}/#session={}", self.address, self.bearer)
    }

    /// Stops the loopback server without stopping the daemon.
    ///
    /// # Cancellation
    ///
    /// Dropping the returned future leaves the server task running.
    pub async fn shutdown(self) -> Result<()> {
        self.task.abort();
        match self.task.await {
            Ok(result) => result,
            Err(error) if error.is_cancelled() => Ok(()),
            Err(error) => Err(anyhow!("Studio server task failed: {error}")),
        }
    }
}

fn ephemeral_bearer() -> Result<String> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .context("open operating-system randomness source")?
        .read_exact(&mut bytes)
        .context("read ephemeral Studio bearer")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
