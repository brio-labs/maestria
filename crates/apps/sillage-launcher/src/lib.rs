//! Native Sillage launcher implemented with Slint and Winit.
//!
//! The launcher owns its resident process and local application index; it has no
//! dependency on the daemon, model runtime, or a webview.
//!
//! Responsibility map:
//! - `ui`: compile and expose the Slint component types.
//! - `application`: coordinate resident native UI and lifecycle.
//! - `ipc`: own generation-safe launcher session state.
//! - `model`: define typed search and action contracts.
//! - `query`: rank applications, commands, and calculations.
//! - `calculator`: evaluate safe arithmetic and offline unit conversions.
//! - `catalog`: discover installed desktop applications.
//! - `actions`: define native command actions.
//! - `settings`: persist launcher preferences.
//! - `search_setup`: own explicit folder consent and the separate read-only search process.
//! - `utilities`: own saved quicklinks/snippets and explicit memory-only clipboard history.
//! - `http_credentials`: scope host-only HTTP credentials in the desktop Secret Service.
//! - `autostart`: manage only the opt-in owned desktop startup entry.
//! - `clipboard`: bound native clipboard reads in an isolated helper process.
//! - `shortcuts`: bind X11 and portal activation.
//! - `platform`: isolate native desktop APIs.
//! - `errors`: define native launcher errors.
//!
//! This module is the stable public facade for the launcher library.
mod ui;
pub use ui::{
    ActionRow, ExtensionActionRow, ExtensionChoiceRow, ExtensionFieldRow, ExtensionItemRow,
    ExtensionRow, HttpIntegrationExtension, HttpIntegrationGrant, LauncherWindow, ResultRow,
    UtilityRow,
};
mod actions;
mod application;
mod autostart;
mod calculator;
mod clipboard;
mod utilities;
pub use clipboard::export_clipboard_text;
mod catalog;
mod errors;
mod http_credentials;
mod ipc;
mod model;
mod platform;
mod query;
mod search_setup;
mod settings;
mod shortcuts;

pub use actions::command_definitions;
pub use application::run;
pub use calculator::{CalculationError, calculate};
pub use catalog::AppEntry;
pub use errors::LauncherError;
pub use ipc::{ActionTarget, LauncherState, QueryWorker};
pub use model::{
    Action, ActionOutcome, CommandDefinition, PreferencesDto, PreferencesUpdate, PrimaryModifier,
    ResultKind, SearchResponse, SearchResult, SearchStatus, SearchStatusKind,
    ShortcutConfigureAction, ShortcutControl, ShortcutSetup, ShortcutState, ShortcutStatus,
    UtilityCollection,
};
pub use query::search_catalog;
pub use settings::SettingsManager;
