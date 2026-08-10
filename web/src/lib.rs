/// Maestria Studio browser client façade.
///
/// Responsibility map:
/// - `api`: browser request and response boundary.
/// - `api_types`: typed browser wire DTOs and Problem Details.
/// - `ask`: question, evidence, and draft-preview interactions.
/// - `app`: root component and route adapters.
/// - `components`: reusable workspace shell and evidence controls.
/// - `drafts`: saved draft editor and revision-safe mutations.
/// - `index`: index choice workspace with candidate tree and policy toggles.
/// - `index_types`: typed wire DTOs for the index choice operations.
/// - `markdown`: safe agent Markdown rendering.
/// - `nav`: global workspace navigation and mobile notebook selector.
/// - `pages`: Dashboard and notebook sections.
/// - `retrieval`: retrieval lane status and promotion records workspace.
/// - `route`: routed workspace paths.
/// - `search`: governed index search workspace.
/// - `session`: bearer handoff and remembered notebook storage.
/// - `state`: epochs, bounded history, and load states.
/// - `tasks`: task list workspace with validation status.
mod api;
mod api_types;
mod app;
mod ask;
mod components;
mod drafts;
mod index;
mod index_types;
mod markdown;
mod nav;
mod pages;
mod retrieval;
mod route;
mod search;
mod session;
mod state;
mod tasks;

pub use api::ApiClient;
pub use app::App;
pub(crate) use app::{
    Index, NotFound, NotebookAsk, NotebookDrafts, NotebookOverview, NotebookSources, Retrieval,
    Search, Tasks,
};
pub use components::WorkspaceContext;
pub(crate) use pages::Dashboard;
