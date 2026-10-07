use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use slint::{ModelRc, VecModel};

use crate::model::SearchResult;
use crate::{ActionRow, LauncherWindow, ResultRow};

const UI_TICK: Duration = Duration::from_millis(50);
// Coalesce typed input without spending a full UI tick before the passage request.
const PASSAGE_SEARCH_DEBOUNCE: Duration = Duration::from_millis(25);
const CATALOG_REFRESH_TICKS: u16 = 600;
const SOURCE_REFRESH_TICKS: u8 = 20;

mod callbacks;
mod dispatch;
mod extensions;
mod passages;
mod platform;
mod preferences;
mod runtime;
mod search;
mod search_setup;
mod source_refresh;
mod utilities;
mod window;

pub use runtime::run;
pub(super) fn empty_actions() -> ModelRc<ActionRow> {
    ModelRc::new(VecModel::default())
}

pub(super) fn empty_results() -> ModelRc<ResultRow> {
    ModelRc::new(VecModel::default())
}

pub(super) type UiWeak = slint::Weak<LauncherWindow>;

pub(super) enum RuntimeMessage {
    Activate,
    Quit,
}

#[derive(Clone)]
pub(super) struct FileSelection {
    result_id: String,
}
#[derive(Clone)]
struct AcceptedPassage {
    result_id: String,
    passage: passages::Passage,
}

#[derive(Clone)]
struct AcceptedPath {
    result_id: String,
    path: String,
}

pub(super) enum DisplayedResult {
    Application(usize),
    Group,
    Passage(usize),
    Path(usize),
}

pub(super) struct FrontendModel {
    query: String,
    passage_search_pending: bool,
    accepted: Vec<SearchResult>,
    selected_file: Option<FileSelection>,
    catalog_ticks_until_refresh: u16,
    accepted_passages: Vec<AcceptedPassage>,
    accepted_paths: Vec<AcceptedPath>,
    passages_loaded: bool,
    displayed: Vec<DisplayedResult>,
    result_filter: String,
    content_view_passages: Vec<usize>,
}

pub(super) struct Frontend {
    generation: AtomicU64,
    active_search: AtomicU64,
    generation_updates: tokio::sync::watch::Sender<u64>,
    interactive_search: tokio::sync::Mutex<()>,
    model: Mutex<FrontendModel>,
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) fn has_argument(arguments: &[String], wanted: &str) -> bool {
    arguments.iter().any(|argument| argument == wanted)
}

#[cfg(target_os = "linux")]
mod instance;

#[cfg(not(target_os = "linux"))]
mod instance {
    use super::RuntimeMessage;
    use std::sync::mpsc;

    pub struct PrimaryInstance;

    impl PrimaryInstance {
        pub fn claim(
            _messages: mpsc::SyncSender<RuntimeMessage>,
            _activate_existing: bool,
        ) -> Result<Option<Self>, Box<dyn std::error::Error>> {
            Ok(Some(Self))
        }
    }

    pub fn send_existing(_message: RuntimeMessage) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(false)
    }
}
