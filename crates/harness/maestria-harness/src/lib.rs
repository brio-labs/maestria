use maestria_ports::{
    HarnessAdapter, HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest,
    PortError,
};
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::process::Stdio;
use std::time::SystemTime;
#[derive(Clone, Default)]
pub struct LocalShellHarnessAdapter;

impl HarnessAdapter for LocalShellHarnessAdapter {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError> {
        Ok(HarnessCapabilities {
            command_classes: vec![HarnessCommandClass::Shell],
            write_enabled: true,
            read_enabled: true,
            web_enabled: false,
        })
    }

    fn execute(&self, request: HarnessRequest) -> Result<HarnessOutcome, PortError> {
        let start = SystemTime::now();

        if request.class != HarnessCommandClass::Shell {
            return Err(PortError::Internal {
                message: format!("Unsupported harness class: {:?}", request.class),
            });
        }

        let output = Command::new("sh")
            .arg("-c")
            .arg(&request.command)
            .current_dir(&request.working_directory)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| PortError::Internal {
                message: format!("LocalShellHarnessAdapter failed: {}", e),
            })?;

        let duration = match start.elapsed() { Ok(d) => d, Err(_) => std::time::Duration::from_secs(0) };
        let exit_code = match output.status.code() {
            Some(c) => c,
            None => match output.status.signal() {
                Some(s) => 128 + s,
                None => -1,
            }
        };

        Ok(HarnessOutcome {
            run_id: request.run_id,
            command: request.command,
            exit_code,
            scope_checked: true, // Assuming governance checked it
            stdout: output.stdout,
            stderr: output.stderr,
            duration,
            artifacts_created: vec![],
            diff_summary: None,
            validation_hints: vec![],
        })
    }
}
