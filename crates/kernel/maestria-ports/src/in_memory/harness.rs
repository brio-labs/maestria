use std::pin::Pin;

use crate::{
    HarnessAdapter, HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest,
    PortError,
};

#[derive(Clone)]
pub struct InMemoryHarnessAdapter {
    capabilities: HarnessCapabilities,
}

impl Default for InMemoryHarnessAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryHarnessAdapter {
    pub fn new() -> Self {
        Self {
            capabilities: HarnessCapabilities {
                command_classes: vec![HarnessCommandClass::Shell, HarnessCommandClass::Browser],
                write_enabled: true,
                read_enabled: true,
                web_enabled: false,
            },
        }
    }
}

impl HarnessAdapter for InMemoryHarnessAdapter {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError> {
        Ok(self.capabilities.clone())
    }

    fn execute(
        &self,
        request: HarnessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HarnessOutcome, PortError>> + Send + '_>> {
        if request.command.trim().is_empty() {
            return Box::pin(std::future::ready(Err(PortError::InvalidInput {
                message: "command must not be empty".to_string(),
            })));
        }

        let mut stdout = Vec::new();
        stdout.extend_from_slice(format!("executed {}", request.command).as_bytes());

        Box::pin(std::future::ready(Ok(HarnessOutcome {
            run_id: request.run_id,
            command: request.command,
            exit_code: 0,
            stdout,
            stderr: Vec::new(),
            duration: std::time::Duration::from_millis(1),
            artifacts_created: Vec::new(),
            diff_summary: None,
            validation_hints: Vec::new(),
        })))
    }
}
