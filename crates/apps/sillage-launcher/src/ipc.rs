mod query_worker;
mod session;

pub use query_worker::QueryWorker;
pub use session::LauncherState;

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum ActionTarget {
    OpenFile,
    ShowPreferences,
    Refresh,
    Quit,
    CopyActivation,
    ResetPreferences,
    OpenSelectedFile(PathBuf),
    CopySelectedPath { path: PathBuf, result_id: String },
    CopyValue(String),
    OpenApplication(String),
}
