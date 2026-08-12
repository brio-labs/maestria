use crate::{
    Dashboard, Index, NotFound, NotebookAsk, NotebookDrafts, NotebookOverview, NotebookSources,
    Retrieval, Search, Tasks,
};
use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq, Routable)]
pub enum Route {
    #[route("/")]
    Dashboard {},
    #[route("/search")]
    Search {},
    #[route("/retrieval")]
    Retrieval {},
    #[route("/tasks")]
    Tasks {},
    #[route("/index")]
    Index {},
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
