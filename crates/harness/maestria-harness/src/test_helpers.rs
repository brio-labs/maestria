use maestria_ports::{HarnessCommandClass, HarnessRequest};
use std::path::PathBuf;
use std::time::Duration;

pub fn adapter() -> crate::LocalShellHarnessAdapter {
    crate::LocalShellHarnessAdapter
}

pub fn shell_request(command: &str, budget_ms: u64) -> HarnessRequest {
    HarnessRequest {
        run_id: maestria_ports::HarnessRunId::new(1),
        command: command.to_string(),
        working_directory: PathBuf::from("/tmp"),
        duration_budget: Duration::from_millis(budget_ms),
        class: HarnessCommandClass::Shell,
        readable_roots: vec![
            PathBuf::from("/"),
            PathBuf::from("/tmp"),
            PathBuf::from("/dev"),
        ],
        blocked_paths: vec![],
        blocked_patterns: vec![],
    }
}
