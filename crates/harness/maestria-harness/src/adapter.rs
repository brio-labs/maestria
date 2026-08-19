use super::command::{self, ALLOWED_PROGRAMS, authorize_paths, validate_command_args};
use super::process::execute_command;
use super::tokenize::tokenize;
use maestria_ports::{
    HarnessAdapter, HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest,
    PortError,
};
use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

#[derive(Clone, Default)]
pub struct LocalShellHarnessAdapter;

impl HarnessAdapter for LocalShellHarnessAdapter {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError> {
        Ok(HarnessCapabilities {
            command_classes: vec![HarnessCommandClass::Shell],
            write_enabled: false,
            read_enabled: cfg!(target_os = "linux"),
            web_enabled: false,
        })
    }

    /// # Cancellation
    /// See [`HarnessAdapter::execute`]: the spawned child is reaped when the
    /// returned future is dropped, and `duration_budget` aborts the run.
    fn execute(
        &self,
        request: HarnessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HarnessOutcome, PortError>> + Send + '_>> {
        Box::pin(execute_impl(request))
    }
}

async fn execute_impl(request: HarnessRequest) -> Result<HarnessOutcome, PortError> {
    let start = SystemTime::now();

    if request.class != HarnessCommandClass::Shell {
        return Err(PortError::InternalContext {
            context: "unsupported harness class",
            source: format!("{:?}", request.class),
        });
    }
    let authorization = authorize_paths(&request)?;
    let mut request = request;
    request.working_directory = authorization.working_directory.clone();

    let argv = tokenize(&request.command)?;
    if argv.is_empty() {
        return Err(PortError::InvalidInputContext {
            context: "validate harness command",
            source: "command must not be empty".to_string(),
        });
    }

    let program = &argv[0];
    if !ALLOWED_PROGRAMS.contains(&program.as_str()) {
        return Err(PortError::InvalidInputContext {
            context: "program not allowed",
            source: program.clone(),
        });
    }

    for arg in &argv {
        command::reject_metachar(arg)?;
    }

    let validated_args = validate_command_args(program, &argv, &request)?;
    let (exit_code, stdout, stderr) =
        execute_command(program, &validated_args, &request, &authorization).await?;

    let duration = start
        .elapsed()
        .map_err(|error| PortError::internal("measure harness run duration", error.to_string()))?;

    Ok(HarnessOutcome {
        run_id: request.run_id,
        command: request.command,
        exit_code,
        stdout,
        stderr,
        duration,
        artifacts_created: vec![],
        diff_summary: None,
        validation_hints: vec![],
    })
}
