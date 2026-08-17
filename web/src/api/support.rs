//! Small wire helpers shared by the ApiClient methods.

use crate::repository_index_types::RepositoryIndexBrowseInputWire;

/// Builds the browse input for a repository-relative directory.
pub(super) fn browse_input(root: &str, path: &str) -> RepositoryIndexBrowseInputWire {
    RepositoryIndexBrowseInputWire {
        root: root.to_string(),
        path: path.to_string(),
    }
}

/// Percent-encodes a path segment for the daemon API routes.
pub(super) fn encode_source_key(key: &str) -> String {
    js_sys::encode_uri_component(key).into()
}

/// Typed notebook bootstrap payload used by the api decode test.
#[cfg(test)]
pub(super) const BOOTSTRAP_JSON: &str = concat!(
    r#"{"status":{"instance_root":"d","event_count":0,"task_count":0},"notebooks":{"notebooks":"#,
    r#"[{"notebook_id":1,"title":"Notes","source_count":0,"updated_at":0}]},"agents":[]}"#,
);
