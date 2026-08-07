/// Maestria Studio browser client façade.
///
/// Responsibility map:
/// - `api`: browser request and response boundary.
/// - `api_types`: typed browser wire DTOs and Problem Details.
/// - `ask`: question, evidence, and draft-preview interactions.
/// - `app`: root component and route adapters.
/// - `components`: reusable workspace shell and evidence controls.
/// - `drafts`: saved draft editor and revision-safe mutations.
/// - `markdown`: safe agent Markdown rendering.
/// - `pages`: Dashboard and notebook sections.
/// - `route`: routed workspace paths.
/// - `session`: bearer handoff and remembered notebook storage.
/// - `state`: epochs, bounded history, and load states.
mod api;
mod api_types;
mod app;
mod ask;
mod components;
mod drafts;
mod markdown;
mod pages;
mod route;
mod session;
mod state;

pub use api::ApiClient;
pub use app::App;
pub(crate) use app::{NotFound, NotebookAsk, NotebookDrafts, NotebookOverview, NotebookSources};
pub use components::WorkspaceContext;
pub(crate) use pages::Dashboard;
