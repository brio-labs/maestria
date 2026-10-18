use crate::model::{
    CommandDefinition, HOST_CLIPBOARD, HOST_OPEN_FILE, HOST_PREFERENCES, HOST_QUICKLINKS,
    HOST_QUIT, HOST_REFRESH_APPLICATIONS, HOST_SNIPPETS,
};

const COMMANDS: [CommandDefinition; 7] = [
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
        title: "Quit Sillage",
        subtitle: "Exit the resident launcher",
        keywords: &["quit", "exit", "close"],
        action_id: "quit",
        action_title: "Quit",
    },
    CommandDefinition {
        id: HOST_QUICKLINKS,
        title: "Quicklinks",
        subtitle: "Manage and open parameterized web links",
        keywords: &["link", "url", "bookmark", "quicklink"],
        action_id: "open",
        action_title: "Open",
    },
    CommandDefinition {
        id: HOST_SNIPPETS,
        title: "Snippets",
        subtitle: "Expand saved text explicitly and copy it",
        keywords: &["snippet", "text", "template", "expand"],
        action_id: "open",
        action_title: "Open",
    },
    CommandDefinition {
        id: HOST_CLIPBOARD,
        title: "Clipboard History",
        subtitle: "Search text explicitly saved for one hour",
        keywords: &["clipboard", "history", "copy", "paste"],
        action_id: "open",
        action_title: "Open",
    },
];

/// Stable host command definitions shared by ranking and action authorization.
pub fn command_definitions() -> &'static [CommandDefinition] {
    &COMMANDS
}
