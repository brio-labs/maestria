mod command;
mod process;
mod tokenize;

use command::{ALLOWED_PROGRAMS, validate_cat_args};
use maestria_ports::{
    HarnessAdapter, HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest,
    PortError,
};
use process::spawn_and_collect;
use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;
pub(crate) use tokenize::tokenize;

// ── adapter ────────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct LocalShellHarnessAdapter;

impl HarnessAdapter for LocalShellHarnessAdapter {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError> {
        Ok(HarnessCapabilities {
            command_classes: vec![HarnessCommandClass::Shell],
            write_enabled: false,
            read_enabled: true,
            web_enabled: false,
        })
    }

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
        return Err(PortError::Internal {
            message: format!("unsupported harness class: {:?}", request.class),
        });
    }

    let argv = tokenize(&request.command)?;
    if argv.is_empty() {
        return Err(PortError::InvalidInput {
            message: "command must not be empty".to_string(),
        });
    }

    let program = &argv[0];
    if !ALLOWED_PROGRAMS.contains(&program.as_str()) {
        return Err(PortError::InvalidInput {
            message: format!(
                "program {:?} not allowed; expected one of {:?}",
                program, ALLOWED_PROGRAMS
            ),
        });
    }

    for arg in &argv {
        command::reject_metachar(arg)?;
    }

    validate_cat_args(program, &argv, &request)?;

    let (status, stdout, stderr) = spawn_and_collect(program, &argv[1..], &request).await?;

    let duration = match start.elapsed() {
        Ok(d) => d,
        Err(_) => std::time::Duration::ZERO,
    };
    let exit_code = status.code().map_or(
        {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                status.signal().map_or(-1, |s| 128 + s)
            }
            #[cfg(not(unix))]
            {
                -1
            }
        },
        |c| c,
    );

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

#[cfg(test)]
mod tests;
