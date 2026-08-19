use dioxus::prelude::*;

use crate::ask::Ask;
use crate::drafts::Drafts;
use crate::{
    api::ApiClient,
    components::{NotebookNav, Shell, WorkspaceContext, alert, set_alert},
    route::Route,
    session::Session,
    state::LoadState,
};

#[derive(Clone, PartialEq)]
pub enum NotebookSection {
    Overview,
    Sources,
    Ask,
    Drafts,
}
#[path = "pages_sources.rs"]
mod pages_sources;
use pages_sources::Sources;

#[component]
pub fn Dashboard() -> Element {
    let context = use_context::<Signal<WorkspaceContext>>();
    let navigator = use_navigator();
    let (agent_label, remembered_notebook, notebooks, alert_error) = {
        let snapshot = context.read();
        let agent_label = if snapshot
            .agent
            .as_ref()
            .is_some_and(|agent| agent.status == "ready")
        {
            "Agent available"
        } else {
            "Agent not configured"
        };
        let remembered_notebook = match (&snapshot.model.notebooks, snapshot.active_notebook) {
            (LoadState::Ready(notebooks), Some(id)) => notebooks
                .iter()
                .find(|notebook| notebook.notebook_id == id)
                .cloned(),
            _ => None,
        };
        let notebooks = snapshot.model.notebooks.clone();
        let alert_error = snapshot.model.alert.clone();
        (agent_label, remembered_notebook, notebooks, alert_error)
    };
    let remembered_title = remembered_notebook
        .as_ref()
        .map(|notebook| notebook.title.clone());
    let remembered_id = remembered_notebook
        .as_ref()
        .map(|notebook| notebook.notebook_id);
    let remembered_source_count = remembered_notebook
        .as_ref()
        .map(|notebook| notebook.source_count);
    rsx! {
        Shell { title: "Workspace", active_notebook: None,
            if let Some(error) = alert_error.as_ref() { {alert(error)} }
            section { class: "mb-6 rounded-lg border border-line bg-panel p-6",
                p {
                    class: "max-w-2xl text-lg text-ink-strong",
                    "A local-first, source-grounded workspace for asking questions and preserving evidence-backed drafts."
                }
                p {
                    class: "mt-2 text-ink-muted",
                    "Your Studio agent and indexed sources stay on this machine."
                }
                p { class: "mt-4 font-medium", {agent_label} }
            }
            if let (Some(notebook_id), Some(title), Some(source_count)) =
                (remembered_id, remembered_title.as_deref(), remembered_source_count)
            {
                section { class: "mb-6 rounded-lg border border-accent bg-panel p-5",
                    div { class: "flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between",
                        div {
                            p { class: "text-sm font-semibold uppercase tracking-wide text-accent",
                                "Continue where you left off"
                            }
                            h2 { class: "mt-1 text-xl font-semibold text-ink-strong", {title} }
                            p { class: "mt-1 text-sm text-ink-muted",
                                "{source_count} selected sources"
                            }
                        }
                        button {
                            class: "rounded bg-accent px-4 py-2 font-semibold text-white hover:bg-accent-hover",
                            onclick: move |_| {
                                navigator.push(Route::NotebookOverview { notebook_id });
                            },
                            "Continue"
                        }
                    }
                }
            }
            NotebookList { notebooks }
        }
    }
}
#[component]
fn NotebookList(notebooks: LoadState<Vec<crate::api::NotebookSummary>>) -> Element {
    let context = use_context::<Signal<WorkspaceContext>>();
    let mut title = use_signal(String::new);
    let navigator = use_navigator();
    rsx! {
        section { class: "rounded-lg border border-line bg-panel p-6",
            h2 { class: "mb-4 text-xl font-semibold text-ink-strong", "Notebooks" }
            match notebooks {
                LoadState::Loading => rsx! { p { "Loading notebooks…" } },
                LoadState::Failed(_) => rsx! { p { "Notebook list unavailable." } },
                LoadState::Empty => rsx! {
                    p {
                        class: "mb-5 text-ink-muted",
                        "Create a notebook to begin"
                    }
                },
                LoadState::Ready(notebooks) => rsx! {
                    div { class: "grid gap-3 md:grid-cols-2",
                        for notebook in notebooks {
                            a {
                                class: "rounded border border-line p-4 hover:border-accent",
                                href: "/notebooks/{notebook.notebook_id}",
                                h3 {
                                    class: "font-semibold text-ink-strong",
                                    "{notebook.title}"
                                }
                                p {
                                    class: "mt-1 text-sm text-ink-muted",
                                    "{notebook.source_count} selected sources"
                                }
                            }
                        }
                    }
                },
            }
            form {
                class: "mt-6 flex flex-col gap-3 sm:flex-row",
                onsubmit: move |event| {
                    event.prevent_default();
                    let value = title.read().trim().to_owned();
                    if value.is_empty() {
                        return;
                    }
                    let mut context = context;
                    let navigator = navigator;
                    spawn(async move {
                        context.write().model.alert = None;
                        match ApiClient::new().create_notebook(value).await {
                            Ok(created) => {
                                Session::remember_notebook(created.notebook_id);
                                navigator.push(Route::NotebookOverview {
                                    notebook_id: created.notebook_id,
                                });
                            }
                            Err(error) => set_alert(context, error),
                        }
                    });
                },
                label { class: "sr-only", r#for: "new-notebook-title", "Notebook title" }
                input {
                    id: "new-notebook-title",
                    class: "flex-1 rounded border border-line bg-input px-3 py-2",
                    placeholder: "Research notes",
                    value: "{title}",
                    oninput: move |event| title.set(event.value())
                }
                button {
                    class: "rounded bg-accent px-4 py-2 font-semibold text-white hover:bg-accent-hover",
                    "Create notebook"
                }
            }
        }
    }
}

#[component]
pub fn NotebookPage(notebook_id: u64, section: NotebookSection) -> Element {
    let context = use_context::<Signal<WorkspaceContext>>();
    let navigator = use_navigator();
    let api = use_hook(ApiClient::new);
    use_effect(move || {
        let mut context = context;
        let api = api.clone();
        Session::remember_notebook(notebook_id);
        spawn(async move {
            let request = {
                let mut value = context.write();
                value.active_notebook = Some(notebook_id);
                value.model.begin_notebook_request(notebook_id)
            };
            let result = async {
                let notebook = api.notebook(notebook_id).await?;
                let sources = api.sources().await?;
                let drafts = api.drafts(notebook_id).await?;
                Ok::<_, crate::api::ClientError>((notebook, sources, drafts))
            }
            .await;
            let mut value = context.write();
            if !value.model.epochs.is_current(request, notebook_id) {
                return;
            }
            match result {
                Ok((notebook, sources, drafts)) => {
                    value.model.notebook = LoadState::Ready(notebook);
                    value.model.sources = LoadState::ready_or_empty(sources);
                    value.model.drafts = LoadState::ready_or_empty(drafts);
                    value.model.alert = None;
                    value.model.status = "Notebook ready".into();
                }
                Err(error) => {
                    value.model.notebook = LoadState::Failed(error.clone());
                    value.model.alert = Some(error);
                    value.model.status = "Action failed".into();
                }
            }
        });
    });
    let (notebook, alert_error, is_loading) = {
        let snapshot = context.read();
        let notebook = match &snapshot.model.notebook {
            LoadState::Ready(value) => Some(value.clone()),
            _ => None,
        };
        let alert_error = snapshot.model.alert.clone();
        let is_loading = matches!(snapshot.model.notebook, LoadState::Loading);
        (notebook, alert_error, is_loading)
    };
    let title = notebook
        .as_ref()
        .map_or_else(|| "Notebook".to_owned(), |value| value.title.clone());
    rsx! {
        Shell { title, active_notebook: Some(notebook_id),
            if let Some(error) = alert_error.as_ref() { {alert(error)} }
            if let Some(notebook) = notebook {
                NotebookNav { notebook: notebook.clone() }
                match section {
                    NotebookSection::Overview => rsx! { Overview { notebook_id, notebook } },
                    NotebookSection::Sources => rsx! { Sources { notebook_id } },
                    NotebookSection::Ask => rsx! { Ask { notebook_id } },
                    NotebookSection::Drafts => rsx! { Drafts { notebook_id } },
                }
            } else if is_loading {
                p { "Loading notebook…" }
            } else {
                p { class: "mb-4", "The requested notebook could not be loaded." }
                button {
                    class: "rounded bg-accent px-4 py-2 text-white",
                    onclick: move |_| {
                        navigator.push(Route::Dashboard {});
                    },
                    "Return to Dashboard"
                }
            }
        }
    }
}

#[component]
fn Overview(notebook_id: u64, notebook: crate::api::Notebook) -> Element {
    let context = use_context::<Signal<WorkspaceContext>>();
    let (available, drafts, agent_status) = {
        let snapshot = context.read();
        let available = match &snapshot.model.sources {
            LoadState::Ready(values) => values.len(),
            _ => 0,
        };
        let drafts = match &snapshot.model.drafts {
            LoadState::Ready(values) => values.len(),
            _ => 0,
        };
        let agent_status = snapshot
            .agent
            .as_ref()
            .map_or("Not configured", |value| value.status.as_str())
            .to_owned();
        (available, drafts, agent_status)
    };
    let selected = notebook
        .sources
        .iter()
        .filter(|source| source.available)
        .count();
    rsx! {
        div { class: "grid gap-4 md:grid-cols-3",
            article {
                class: "rounded-lg border border-line bg-panel p-5",
                h3 { class: "font-semibold", "Selected sources" }
                p {
                    class: "mt-2 text-3xl font-bold text-ink-strong",
                    "{selected}"
                }
                p {
                    class: "text-sm text-ink-muted",
                    "of {available} available"
                }
            }
            article {
                class: "rounded-lg border border-line bg-panel p-5",
                h3 { class: "font-semibold", "Saved drafts" }
                p {
                    class: "mt-2 text-3xl font-bold text-ink-strong",
                    "{drafts}"
                }
            }
            article {
                class: "rounded-lg border border-line bg-panel p-5",
                h3 { class: "font-semibold", "Agent" }
                p { class: "mt-2", {agent_status} }
            }
        }
        div {
            class: "mt-6 flex flex-wrap gap-3",
            a {
                class: "rounded bg-accent px-4 py-2 text-white",
                href: "/notebooks/{notebook_id}/sources",
                "Manage sources"
            }
            a {
                class: "rounded border border-line bg-panel px-4 py-2",
                href: "/notebooks/{notebook_id}/ask",
                "Ask a question"
            }
            a {
                class: "rounded border border-line bg-panel px-4 py-2",
                href: "/notebooks/{notebook_id}/drafts",
                "Open drafts"
            }
        }
    }
}
