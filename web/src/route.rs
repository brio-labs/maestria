use crate::{Dashboard, NotFound, NotebookAsk, NotebookDrafts, NotebookOverview, NotebookSources};
use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq, Routable)]
pub enum Route {
    #[route("/")]
    Dashboard {},
    #[route("/notebooks/:notebook_id")]
    NotebookOverview { notebook_id: u64 },
    #[route("/notebooks/:notebook_id/sources")]
    NotebookSources { notebook_id: u64 },
    #[route("/notebooks/:notebook_id/ask")]
    NotebookAsk { notebook_id: u64 },
    #[route("/notebooks/:notebook_id/drafts")]
    NotebookDrafts { notebook_id: u64 },
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}
