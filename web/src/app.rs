use dioxus::prelude::*;

use crate::{api::ApiClient, components::WorkspaceContext, pages::NotebookPage};

#[component]
pub(crate) fn NotebookOverview(notebook_id: u64) -> Element {
    rsx! { NotebookPage { notebook_id, section: "overview" } }
}

#[component]
pub(crate) fn NotebookSources(notebook_id: u64) -> Element {
    rsx! { NotebookPage { notebook_id, section: "sources" } }
}

#[component]
pub(crate) fn NotebookAsk(notebook_id: u64) -> Element {
    rsx! { NotebookPage { notebook_id, section: "ask" } }
}

#[component]
pub(crate) fn NotebookDrafts(notebook_id: u64) -> Element {
    rsx! { NotebookPage { notebook_id, section: "drafts" } }
}

#[component]
pub(crate) fn Search() -> Element {
    rsx! { crate::search::SearchWorkspace {} }
}

#[component]
pub(crate) fn Retrieval() -> Element {
    rsx! { crate::retrieval::RetrievalWorkspace {} }
}

#[component]
pub(crate) fn Tasks() -> Element {
    rsx! { crate::tasks::TasksWorkspace {} }
}

#[component]
pub(crate) fn Index() -> Element {
    rsx! { crate::index::IndexWorkspace {} }
}

#[component]
pub(crate) fn IndexRepositories() -> Element {
    rsx! { crate::repositories::RepositoryIndexWorkspace {} }
}

#[component]
pub(crate) fn NotFound(segments: Vec<String>) -> Element {
    let _ = segments;
    rsx! {
        div { class: "min-h-screen bg-page p-8",
            h1 { class: "text-2xl font-bold", "Not found" }
            a {
                class: "mt-4 inline-block rounded bg-accent px-4 py-2 text-white",
                href: "/",
                "Return to Dashboard"
            }
        }
    }
}

/// Pick the agent that answers notebook questions: the remembered agent when
/// it is still configured, else the first ready agent, else the first agent.
fn preferred_agent(
    agents: &[crate::api::Agent],
    remembered: Option<String>,
) -> Option<crate::api::Agent> {
    if let Some(id) = remembered
        && let Some(agent) = agents.iter().find(|agent| agent.id == id)
    {
        return Some(agent.clone());
    }
    agents
        .iter()
        .find(|agent| agent.status == "ready")
        .or_else(|| agents.first())
        .cloned()
}

#[component]
pub fn App() -> Element {
    let context = use_context_provider(|| Signal::new(WorkspaceContext::default()));
    let client = use_hook(ApiClient::new);
    use_effect(move || {
        let mut context = context;
        let client = client.clone();
        spawn(async move {
            context.write().model.status = "Loading Studio…".into();
            match client.bootstrap().await {
                Ok(bootstrap) => {
                    let notebooks = bootstrap.notebooks.into_vec();
                    let remembered = crate::session::Session::remembered_notebook();
                    let remembered = remembered
                        .filter(|id| notebooks.iter().any(|notebook| notebook.notebook_id == *id));
                    if crate::session::Session::remembered_notebook().is_some()
                        && remembered.is_none()
                    {
                        crate::session::Session::clear_notebook();
                    }
                    let mut value = context.write();
                    value.model.notebooks = if notebooks.is_empty() {
                        crate::state::LoadState::Empty
                    } else {
                        crate::state::LoadState::Ready(notebooks)
                    };
                    value.model.agents = bootstrap.agents.clone();
                    value.agent = preferred_agent(
                        &bootstrap.agents,
                        crate::session::Session::remembered_agent(),
                    );
                    value.bootstrap_status = bootstrap.status.clone();
                    value.model.status = "Studio ready".into();
                    value.active_notebook = remembered;
                }
                Err(error) => {
                    let mut value = context.write();
                    value.model.notebooks = crate::state::LoadState::Failed(error.clone());
                    value.model.alert = Some(error);
                    value.model.status = "Action failed".into();
                }
            }
        });
    });
    rsx! {
        document::Link { rel: "stylesheet", href: "/assets/tailwind.css" }
        Router::<crate::route::Route> {}
    }
}
