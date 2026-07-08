use maestria_ports::{
    HarnessAdapter, HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest,
    PortError,
};
use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::time::SystemTime;
use tokio::process::Command;
use tokio::time::timeout;
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

        let budget = request.duration_budget;
        let cmd_string = request.command.clone();
        let work_dir = request.working_directory.clone();

        let output_result = match std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => {
                    return Err(PortError::Internal {
                        message: "Failed to build tokio runtime".to_string(),
                    })
                }
            };

            Ok(rt.block_on(async move {
                let mut cmd = Command::new("sh");
                cmd.arg("-c")
                    .arg(&cmd_string)
                    .current_dir(&work_dir)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                timeout(budget, cmd.output()).await
            }))
        })
        .join()
        {
            Ok(res) => res,
            Err(_) => Err(PortError::Internal {
                message: "Thread panicked".to_string(),
            }),
        };

        let output = match output_result {
            Ok(Ok(Ok(output))) => output,
            Ok(Ok(Err(e))) => {
                return Err(PortError::Internal {
                    message: format!("LocalShellHarnessAdapter failed: {}", e),
                })
            }
            Ok(Err(_)) => {
                return Err(PortError::Internal {
                    message: "LocalShellHarnessAdapter timed out".to_string(),
                })
            }
            Err(e) => return Err(e),
        };

        let duration = match start.elapsed() {
            Ok(d) => d,
            Err(_) => std::time::Duration::from_secs(0),
        };
        let exit_code = match output.status.code() {
            Some(c) => c,
            None => match output.status.signal() {
                Some(s) => 128 + s,
                None => -1,
            },
        };

        Ok(HarnessOutcome {
            run_id: request.run_id,
            command: request.command,
            exit_code,
            scope_checked: false, // The adapter executes raw commands; scope is checked upstream by Governance.
            stdout: output.stdout,
            stderr: output.stderr,
            duration,
            artifacts_created: vec![],
            diff_summary: None,
            validation_hints: vec![],
        })
    }
}
