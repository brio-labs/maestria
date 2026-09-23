use serde::{Deserialize, Serialize};

pub const LAUNCHER_WINDOW_LABEL: &str = "launcher";
pub const HOST_OPEN_FILE: &str = "host.open-file";
pub const HOST_PREFERENCES: &str = "host.preferences";
pub const HOST_REFRESH_APPLICATIONS: &str = "host.refresh-applications";
pub const HOST_QUIT: &str = "host.quit";
pub const HOST_COPY_ACTIVATION: &str = "host.copy-activation";
pub const HOST_RESET_PREFERENCES: &str = "host.reset-preferences";
pub const SELECTED_FILE_PREFIX: &str = "host.file.";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub generation: u64,
    pub catalog_revision: u64,
    pub results: Vec<SearchResult>,
    pub status: SearchStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchStatus {
    pub kind: SearchStatusKind,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchStatusKind {
    Loading,
    Ready,
    Refreshing,
    Error,
    CalculationError,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub id: String,
    pub kind: ResultKind,
    pub title: String,
    pub subtitle: String,
    pub icon: Option<String>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResultKind {
    Application,
    Command,
    Calculation,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: String,
    pub title: String,
    pub primary: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum ActionOutcome {
    #[serde(rename = "dismiss")]
    Dismiss,
    #[serde(rename = "copied")]
    Copied {
        #[serde(rename = "resultId", skip_serializing_if = "Option::is_none")]
        result_id: Option<String>,
    },
    #[serde(rename = "show_preferences")]
    ShowPreferences,
    #[serde(rename = "file_selected")]
    FileSelected {
        #[serde(rename = "resultId")]
        result_id: String,
        #[serde(rename = "displayPath")]
        display_path: String,
    },
    #[serde(rename = "refreshed")]
    Refreshed,
    #[serde(rename = "cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesDto {
    pub schema_version: u32,
    pub shortcut: String,
    pub shortcut_setup: ShortcutSetup,
    pub reduce_motion: bool,
    pub warning: Option<String>,
    pub read_only: bool,
    pub platform: String,
    pub accelerators: AcceleratorPresentation,
    pub shortcut_status: ShortcutStatus,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceleratorPresentation {
    pub primary_modifier: PrimaryModifier,
    pub primary_label: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrimaryModifier {
    Control,
    Meta,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShortcutSetup {
    Unconfigured,
    Requested,
    Deferred,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShortcutState {
    Unconfigured,
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShortcutControl {
    Application,
    System,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShortcutConfigureAction {
    Setup,
    Change,
    Rebind,
    Retry,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutStatus {
    pub state: ShortcutState,
    pub description: String,
    pub message: Option<String>,
    pub control: ShortcutControl,
    pub configure_action: ShortcutConfigureAction,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesUpdate {
    pub reduce_motion: Option<bool>,
    pub shortcut: Option<String>,
    pub shortcut_setup: Option<ShortcutSetup>,
    pub confirm_reset: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct CommandDefinition {
    pub id: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub keywords: &'static [&'static str],
    pub action_id: &'static str,
    pub action_title: &'static str,
}
