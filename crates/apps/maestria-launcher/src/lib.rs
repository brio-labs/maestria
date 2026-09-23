//! Responsibility map:
//!
//! - `application`: resident Tauri window construction and process lifecycle.
//! - `ipc`: window-scoped typed renderer boundary, accepted IDs, and query worker.
//! - `model`: serializable camelCase DTOs and stable host identifiers.
//! - `query`: deterministic application/command matching and ranking.
//! - `calculator`: bounded finite decimal arithmetic, never code evaluation.
//! - `catalog`: owned snapshots and coalesced background discovery.
//! - `actions`: built-in command registration and matching.
//! - `settings`: native TOML persistence and reset/read-only policy.
//! - `shortcuts`: native shortcut registration, portal sessions and activation.
//! - `metrics`: opt-in, content-free native and renderer-ready timing.
//! - `platform`: cfg-selected Linux discovery, launch, icons and display integration.
//! - `errors`: structured recoverable launcher failures.
mod actions;
mod application;
mod calculator;
mod catalog;
mod errors;
mod ipc;
mod metrics;
mod model;
mod platform;
mod query;
mod settings;
mod shortcuts;

pub use actions::command_definitions;
pub(crate) use application::request_activation;
pub use application::run;
pub use calculator::{CalculationError, calculate};
pub use catalog::AppEntry;
pub use ipc::{ActionTarget, LauncherState, QueryWorker};
pub use model::{
    AcceleratorPresentation, Action, ActionOutcome, CommandDefinition, HOST_COPY_ACTIVATION,
    HOST_OPEN_FILE, HOST_PREFERENCES, HOST_QUIT, HOST_REFRESH_APPLICATIONS, HOST_RESET_PREFERENCES,
    LAUNCHER_WINDOW_LABEL, PreferencesDto, PreferencesUpdate, PrimaryModifier, ResultKind,
    SearchResponse, SearchResult, SearchStatus, SearchStatusKind, ShortcutConfigureAction,
    ShortcutControl, ShortcutSetup, ShortcutState, ShortcutStatus,
};
pub use query::search_catalog;
pub use settings::SettingsManager;
