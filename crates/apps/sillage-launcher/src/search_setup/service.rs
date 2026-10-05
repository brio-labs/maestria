use super::*;

pub(super) async fn run_cli(
    program: &Path,
    label: &'static str,
    args: Vec<std::ffi::OsString>,
    limit: Duration,
) -> Result<CommandOutput, String> {
    if limit.is_zero() {
        return Err("Managed search shutdown exceeded its five-second deadline.".to_string());
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir("/")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|_| format!("Could not run the Sillage search command: {label}."))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("Could not capture Sillage search output for {label}."))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("Could not drain Sillage search diagnostics for {label}."))?;
    let result = timeout(limit, async {
        let wait = child.wait();
        tokio::pin!(wait);
        let (status, stdout, _stderr) = tokio::join!(
            wait,
            drain_bounded(stdout, MAX_OUTPUT_BYTES),
            drain_bounded(stderr, MAX_OUTPUT_BYTES),
        );
        let status = status.map_err(|_| format!("Sillage search command failed: {label}."))?;
        let stdout =
            stdout.map_err(|_| format!("Could not read Sillage search output for {label}."))?;
        let _ = _stderr
            .map_err(|_| format!("Could not drain Sillage search diagnostics for {label}."))?;
        if !status.success() {
            return Err(format!("Sillage search command {label} failed ({status})."));
        }
        Ok(CommandOutput { stdout })
    })
    .await;
    match result {
        Ok(output) => output,
        Err(_) => Err(format!("Sillage search command {label} timed out.")),
    }
}

async fn drain_bounded(mut reader: impl AsyncRead + Unpin, limit: usize) -> io::Result<Vec<u8>> {
    let mut retained = Vec::with_capacity(limit.min(8192));
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(retained.len());
        retained.extend_from_slice(&chunk[..read.min(remaining)]);
    }
    Ok(retained)
}
pub(super) fn socket_path_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("The managed search socket path is occupied by a symbolic link.".to_string())
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("Could not inspect the managed search socket path.".to_string()),
    }
}
pub(super) fn socket_identity(path: &Path) -> Result<SocketIdentity, String> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| "The managed search socket is unavailable.".to_string())?;
        if !metadata.file_type().is_socket() {
            return Err("The managed search endpoint is not a Unix socket.".to_string());
        }
        Ok(SocketIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err("Managed read-only search is currently supported only on Linux.".to_string())
    }
}
pub(super) async fn wait_for_socket_absence(
    path: &Path,
    expected: Option<SocketIdentity>,
    deadline: MonotonicInstant,
) -> bool {
    loop {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return true,
            Ok(_)
                if expected
                    .is_some_and(|expected| socket_identity(path).ok() == Some(expected)) =>
            {
                if MonotonicInstant::now() >= deadline {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            _ => return false,
        }
    }
}
impl SearchSetup {
    pub(super) async fn cli(
        &self,
        generation: u64,
        program: &Path,
        label: &'static str,
        args: Vec<std::ffi::OsString>,
    ) -> Result<CommandOutput, String> {
        let mut cancelled = self.intent.subscribe();
        if *cancelled.borrow() != generation {
            return Err("Managed search setup was cancelled by a newer user action.".to_string());
        }
        tokio::select! {
            biased;
            changed = cancelled.changed() => {
                let _ = changed;
                Err("Managed search setup was cancelled by a newer user action.".to_string())
            }
            output = run_cli(program, label, args, COMMAND_TIMEOUT) => output,
        }
    }

    pub(super) async fn run_owned_cli(
        &self,
        owned: &mut OwnedState,
        generation: Option<u64>,
        deadline: Option<MonotonicInstant>,
        program: &Path,
        label: &'static str,
        args: Vec<std::ffi::OsString>,
    ) -> Result<CommandOutput, String> {
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(MonotonicInstant::now());
            if remaining.is_zero() {
                return Err(
                    "Managed search shutdown exceeded its five-second deadline.".to_string()
                );
            }
            run_cli(program, label, args, remaining.min(COMMAND_TIMEOUT)).await
        } else if let Some(generation) = generation {
            self.cli(generation, program, label, args).await
        } else {
            run_cli(program, label, args, COMMAND_TIMEOUT).await
        }
        .and_then(|output| {
            self.ensure_live_endpoint(owned)?;
            Ok(output)
        })
    }

    pub(super) async fn start_owned_service(
        &self,
        owned: &mut OwnedState,
        program: &Path,
        profile_root: &Path,
        socket_path: &Path,
        generation: u64,
    ) -> Result<(), String> {
        self.check_intent(generation)?;
        if socket_path_exists(socket_path)? {
            return Err("A service is already bound to the managed search socket; refusing to attach or replace it.".to_string());
        }
        let mut command = Command::new(program);
        command
            .arg("start")
            .arg("--instance-dir")
            .arg(profile_root)
            .current_dir("/")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command
            .spawn()
            .map_err(|_| "Could not start the standalone Sillage search component.".to_string())?;
        owned.child = Some(child);
        owned.profile_root = Some(profile_root.to_path_buf());
        owned.socket_path = Some(socket_path.to_path_buf());
        owned.program = Some(program.to_path_buf());
        owned.socket_identity = None;

        let deadline = MonotonicInstant::now() + STARTUP_TIMEOUT;
        loop {
            self.check_intent(generation)?;
            if owned
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten())
                .is_some()
            {
                return Err("The managed Sillage search service exited during startup.".to_string());
            }
            match socket_identity(socket_path) {
                Ok(identity) => {
                    owned.socket_identity = Some(identity);
                    return Ok(());
                }
                Err(_) if MonotonicInstant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(_) => {
                    return Err(
                        "The managed Sillage search service did not create its private socket."
                            .to_string(),
                    );
                }
            }
        }
    }

    pub(super) fn ensure_live_endpoint(&self, owned: &mut OwnedState) -> Result<(), String> {
        let Some(child) = owned.child.as_mut() else {
            return Err("The Sillage search service is not owned by this launcher.".to_string());
        };
        match child.try_wait() {
            Ok(Some(_)) => {
                return Err("The managed Sillage search service is not running.".to_string());
            }
            Ok(None) => {}
            Err(_) => return Err("The managed Sillage process state is unavailable.".to_string()),
        }
        let socket = owned
            .socket_path
            .as_deref()
            .ok_or_else(|| "The managed search socket path is unavailable.".to_string())?;
        let identity = socket_identity(socket).map_err(|_| {
            "The managed Sillage search service is not accepting requests.".to_string()
        })?;
        if owned
            .socket_identity
            .is_some_and(|expected| expected != identity)
        {
            return Err(
                "The managed search socket changed; refusing to contact an unowned service."
                    .to_string(),
            );
        }
        owned.socket_identity = Some(identity);
        Ok(())
    }

    pub(super) async fn stop_owned_child(
        &self,
        owned: &mut OwnedState,
        deadline: MonotonicInstant,
    ) -> Result<(), String> {
        let Some(child) = owned.child.as_mut() else {
            return Ok(());
        };
        let already_exited = child
            .try_wait()
            .map_err(|_| "Could not inspect the owned Sillage search process.".to_string())?
            .is_some();
        if !already_exited {
            self.signal_owned_child(owned)?;
        }
        if !already_exited {
            let remaining = deadline.saturating_duration_since(MonotonicInstant::now());
            if remaining.is_zero() {
                return Err(
                    "Managed search shutdown exceeded its five-second deadline.".to_string()
                );
            }
            let child = owned
                .child
                .as_mut()
                .ok_or_else(|| "The owned Sillage search process is unavailable.".to_string())?;
            timeout(remaining, child.wait())
                .await
                .map_err(|_| {
                    "The owned Sillage search service did not exit within five seconds.".to_string()
                })?
                .map_err(|_| {
                    "Could not wait for the owned Sillage search service to exit.".to_string()
                })?;
        }
        let socket = owned.socket_path.clone();
        let expected_socket = owned.socket_identity;
        owned.child = None;
        owned.socket_identity = None;
        owned.profile_lock = None;
        if let Some(socket) = socket
            && !wait_for_socket_absence(&socket, expected_socket, deadline).await
        {
            return Err("The owned Sillage search service exited but its socket path is no longer safely absent.".to_string());
        }
        Ok(())
    }

    fn signal_owned_child(&self, owned: &mut OwnedState) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            let pid = owned
                .child
                .as_ref()
                .and_then(|child| child.id())
                .ok_or_else(|| {
                    "The owned Sillage search process has no signalable process id.".to_string()
                })?;
            if unsafe { libc::kill(pid as libc::pid_t, libc::SIGINT) } != 0
                && owned
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok().flatten())
                    .is_none()
            {
                return Err(
                    "Could not request normal SIGINT shutdown of the owned Sillage search service."
                        .to_string(),
                );
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = owned;
            Err("Graceful managed search shutdown is supported only on Linux.".to_string())
        }
    }
}
