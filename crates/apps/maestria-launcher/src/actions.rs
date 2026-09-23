use crate::model::{
    CommandDefinition, HOST_OPEN_FILE, HOST_PREFERENCES, HOST_QUIT, HOST_REFRESH_APPLICATIONS,
};

const COMMANDS: [CommandDefinition; 4] = [
    CommandDefinition {
        id: HOST_OPEN_FILE,
        title: "Open File…",
        subtitle: "Choose a local file",
        keywords: &["open", "file", "document", "path"],
        action_id: "open",
        action_title: "Open",
    },
    CommandDefinition {
        id: HOST_PREFERENCES,
        title: "Preferences",
        subtitle: "Configure launcher preferences",
        keywords: &["settings", "preferences", "shortcut", "motion"],
        action_id: "open",
        action_title: "Open",
    },
    CommandDefinition {
        id: HOST_REFRESH_APPLICATIONS,
        title: "Refresh Applications",
        subtitle: "Reload installed applications",
        keywords: &["refresh", "reload", "applications", "apps"],
        action_id: "refresh",
        action_title: "Refresh",
    },
    CommandDefinition {
        id: HOST_QUIT,
        title: "Quit Maestria",
        subtitle: "Exit the resident launcher",
        keywords: &["quit", "exit", "close"],
        action_id: "quit",
        action_title: "Quit",
    },
];

/// Stable host command definitions shared by ranking and action authorization.
pub fn command_definitions() -> &'static [CommandDefinition] {
    &COMMANDS
}
