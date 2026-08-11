use dioxus::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

use crate::{
    api::{
        Agent, ApiClient, BootstrapStatus, Citation, ClientError, Draft, DraftSummary, Evidence,
        Notebook,
    },
    route::Route,
    session::Session,
    state::{LoadState, StudioStateModel},
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceContext {
    pub model: StudioStateModel,
    pub active_notebook: Option<u64>,
    pub agent: Option<Agent>,
    pub evidence: Option<Evidence>,
    pub invoking_citation: Option<u64>,
    pub bootstrap_status: Option<BootstrapStatus>,
}

pub fn alert(error: &ClientError) -> Element {
    rsx! {
        div {
            role: "alert",
            class: "rounded border border-danger bg-danger-soft p-3 text-danger",
            strong { "{error.title()}" }
            p { "{error.detail()}" }
        }
    }
}

#[component]
pub fn Shell(title: String, active_notebook: Option<u64>, children: Element) -> Element {
    let context = use_context::<Signal<WorkspaceContext>>();
    let snapshot = context.read().clone();
    let notebooks = match snapshot.model.notebooks {
        LoadState::Ready(items) => items,
        _ => Vec::new(),
    };
    let agent_ready = snapshot
        .agent
        .as_ref()
        .is_some_and(|agent| agent.status == "ready");
    let selected_notebook = active_notebook.map_or_else(String::new, |id| id.to_string());
    rsx! {
        div { class: "min-h-screen bg-page text-ink",
            header { class: "border-b border-line bg-panel px-4 py-3 md:px-8",
                div { class: "mx-auto flex max-w-7xl items-center justify-between gap-4",
                    a { href: "/", class: "text-lg font-bold text-ink-strong", "Maestria Studio" }
                    div { class: "flex items-center gap-3 text-sm",
                        span {
                            class: if agent_ready { "text-success" } else { "text-warning" },
                            aria_label: "Agent readiness",
                            "● "
                            {if agent_ready { "Agent ready" } else { "Agent unavailable" }}
                        }
                        span { role: "status", aria_live: "polite", "{snapshot.model.status}" }
                    }
                }
            }
                div { class: "mb-4 lg:hidden",
                    crate::nav::NotebookSelector {
                        notebooks: notebooks.clone(),
                        selected_notebook: selected_notebook.clone()
                    }
                }
            div { class: "mx-auto flex max-w-7xl gap-6 p-4 md:p-8",
                crate::nav::GlobalNav { notebooks }
                main {
                    class: "min-w-0 flex-1",
                    h1 {
                        class: "mb-6 text-2xl font-bold text-ink-strong",
                        {title},
                    },
                    {children}
                }
            }
        }
    }
}
fn record_error(mut context: Signal<WorkspaceContext>, error: ClientError) {
    let mut value = context.write();
    value.model.alert = Some(error);
    value.model.status = "Action failed".into();
}

fn open_dialog(id: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
    #[cfg(target_arch = "wasm32")]
    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        && let Ok(dialog) = element.dyn_into::<web_sys::HtmlDialogElement>()
    {
        let _ = dialog.show_modal();
    }
}

fn close_dialog(id: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
    #[cfg(target_arch = "wasm32")]
    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        && let Ok(dialog) = element.dyn_into::<web_sys::HtmlDialogElement>()
    {
        dialog.close();
    }
}

#[component]
pub fn NotebookNav(notebook: Notebook) -> Element {
    let id = notebook.notebook_id;
    let context = use_context::<Signal<WorkspaceContext>>();
    let api = use_hook(ApiClient::new);
    let selected_count = notebook
        .sources
        .iter()
        .filter(|source| source.available)
        .count();
    let mut rename_open = use_signal(|| false);
    let mut delete_open = use_signal(|| false);
    let mut title = use_signal(|| notebook.title.clone());
    rsx! {
        div { class: "mb-6 rounded-lg border border-line bg-panel p-4",
            div { class: "flex flex-wrap items-center justify-between gap-3",
                div {
                    h2 { class: "text-xl font-semibold text-ink-strong", "{notebook.title}" }
                    span { class: "text-sm text-ink-muted", "{selected_count} selected sources" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "rounded border border-line px-3 py-2",
                        onclick: move |_| {
                            title.set(notebook.title.clone());
                            rename_open.set(true);
                            open_dialog("rename-notebook-dialog");
                        },
                        "Rename"
                    }
                    button {
                        class: "rounded border border-danger px-3 py-2 text-danger",
                        onclick: move |_| {
                            delete_open.set(true);
                            open_dialog("delete-notebook-dialog");
                        },
                        "Delete"
                    }
                }
            }
            nav { class: "mt-4 flex gap-2 overflow-x-auto", aria_label: "Notebook sections",
                a {
                    class: "whitespace-nowrap rounded px-3 py-2 hover:bg-muted",
                    href: "/notebooks/{id}",
                    "Overview"
                }
                a {
                    class: "whitespace-nowrap rounded px-3 py-2 hover:bg-muted",
                    href: "/notebooks/{id}/sources",
                    "Sources"
                }
                a {
                    class: "whitespace-nowrap rounded px-3 py-2 hover:bg-muted",
                    href: "/notebooks/{id}/ask",
                    "Ask"
                }
                a {
                    class: "whitespace-nowrap rounded px-3 py-2 hover:bg-muted",
                    href: "/notebooks/{id}/drafts",
                    "Drafts"
                }
            }
            RenameNotebookDialog {
                notebook_id: id,
                context,
                api: api.clone(),
                open: rename_open,
                title
            }
            DeleteNotebookDialog {
                notebook_id: id,
                context,
                api,
                open: delete_open
            }
        }
    }
}

#[component]
fn RenameNotebookDialog(
    notebook_id: u64,
    context: Signal<WorkspaceContext>,
    api: ApiClient,
    mut open: Signal<bool>,
    mut title: Signal<String>,
) -> Element {
    rsx! {
        dialog { id: "rename-notebook-dialog", aria_labelledby: "rename-notebook-title",
            h3 {
                id: "rename-notebook-title",
                class: "text-lg font-semibold",
                "Rename notebook"
            }
            form {
                onsubmit: move |event| {
                    event.prevent_default();
                    let next_title = title.read().trim().to_owned();
                    if next_title.is_empty() {
                        return;
                    }
                    let api = api.clone();
                    spawn(async move {
                        match api.rename_notebook(notebook_id, next_title).await {
                            Ok(updated) => match api.bootstrap().await {
                                Ok(bootstrap) => {
                                    let notebooks = bootstrap.notebooks.into_vec();
                                    let mut value = context.write();
                                    value.model.notebooks = if notebooks.is_empty() {
                                        LoadState::Empty
                                    } else {
                                        LoadState::Ready(notebooks)
                                    };
                                    value.model.notebook = LoadState::Ready(updated);
                                    value.model.alert = None;
                                    value.model.status = "Notebook renamed".into();
                                    open.set(false);
                                    close_dialog("rename-notebook-dialog");
                                }
                                Err(error) => record_error(context, error),
                            },
                            Err(error) => record_error(context, error),
                        }
                    });
                },
                label {
                    class: "mt-4 block text-sm font-medium",
                    r#for: "notebook-title",
                    "Title"
                }
                input {
                    id: "notebook-title",
                    class: "mt-1 w-full rounded border border-line bg-input px-3 py-2",
                    value: "{title}",
                    oninput: move |event| title.set(event.value())
                }
                div { class: "mt-4 flex justify-end gap-2",
                    button {
                        r#type: "button",
                        class: "rounded border border-line px-3 py-2",
                        onclick: move |_| {
                            open.set(false);
                            close_dialog("rename-notebook-dialog");
                        },
                        "Cancel"
                    }
                    button { class: "rounded bg-accent px-3 py-2 text-white", "Save" }
                }
            }
        }
    }
}

#[component]
fn DeleteNotebookDialog(
    notebook_id: u64,
    context: Signal<WorkspaceContext>,
    api: ApiClient,
    mut open: Signal<bool>,
) -> Element {
    let navigator = use_navigator();
    rsx! {
        dialog { id: "delete-notebook-dialog", aria_labelledby: "delete-notebook-title",
            h3 {
                id: "delete-notebook-title",
                class: "text-lg font-semibold",
                "Delete notebook?"
            }
            p {
                class: "mt-2 text-ink-muted",
                "This removes the notebook and its saved drafts."
            }
            div { class: "mt-4 flex justify-end gap-2",
                button {
                    class: "rounded border border-line px-3 py-2",
                    onclick: move |_| {
                        open.set(false);
                        close_dialog("delete-notebook-dialog");
                    },
                    "Cancel"
                }
                button {
                    class: "rounded bg-danger px-3 py-2 text-white",
                    onclick: move |_| {
                        let api = api.clone();
                        spawn(async move {
                            match api.delete_notebook(notebook_id).await {
                                Ok(()) => match api.bootstrap().await {
                                    Ok(bootstrap) => {
                                        let notebooks = bootstrap.notebooks.into_vec();
                                        let mut value = context.write();
                                        value.model.notebooks = if notebooks.is_empty() {
                                            LoadState::Empty
                                        } else {
                                            LoadState::Ready(notebooks)
                                        };
                                        value.model.alert = None;
                                        value.model.status = "Notebook deleted".into();
                                        Session::clear_notebook();
                                        open.set(false);
                                        close_dialog("delete-notebook-dialog");
                                        navigator.push(Route::Dashboard {});
                                    }
                                    Err(error) => record_error(context, error),
                                },
                                Err(error) => record_error(context, error),
                            }
                        });
                    },
                    "Delete"
                }
            }
        }
    }
}
#[component]
pub fn DraftButton(
    notebook_id: u64,
    draft: DraftSummary,
    api: ApiClient,
    mut title: Signal<String>,
    mut markdown: Signal<String>,
    mut selected: Signal<Option<Draft>>,
    context: Signal<WorkspaceContext>,
) -> Element {
    let draft_id = draft.draft_id;
    // Snapshot the editor state at click time; a slower draft fetch must
    // not clobber edits the user makes while it is in flight.
    let prior_title = title.read().clone();
    let prior_markdown = markdown.read().clone();
    rsx! {
        button {
            class: "mb-2 block w-full rounded border border-line p-3 text-left",
            onclick: move |_| {
                let api = api.clone();
                let prior_title = prior_title.clone();
                let prior_markdown = prior_markdown.clone();
                spawn(async move {
                    match api.draft(notebook_id, draft_id).await {
                        Ok(value) => {
                            if title.read().as_str() == prior_title.as_str()
                                && markdown.read().as_str() == prior_markdown.as_str()
                            {
                                title.set(value.title.clone());
                                markdown.set(value.markdown.clone());
                            }
                            selected.set(Some(value));
                        }
                        Err(error) => {
                            let mut state = context.write();
                            state.model.alert = Some(error);
                            state.model.status = "Draft load failed".into();
                        }
                    }
                });
            },
            "{draft.title}"
            p { class: "text-sm text-ink-muted", "Revision {draft.revision}" }
        }
    }
}

#[component]
pub fn CitationList(citations: Vec<Citation>, on_open: EventHandler<u64>) -> Element {
    rsx! {
        div { class: "space-y-2",
            for citation in citations {
                button {
                    class: "block w-full rounded border border-line bg-panel p-3 text-left hover:border-accent",
                    onclick: move |_| on_open.call(citation.evidence.evidence_id),
                    "[{citation.rank}] {citation.evidence.artifact_title}"
                    p {
                        class: "mt-1 line-clamp-2 text-sm text-ink-muted",
                        "{citation.evidence.excerpt}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn EvidenceDialog(evidence: Option<Evidence>, on_close: EventHandler<()>) -> Element {
    let Some(evidence) = evidence else {
        return rsx! {};
    };
    rsx! {
        div {
            class: "fixed inset-0 z-10 flex items-end justify-end bg-black/30",
            role: "dialog",
            aria_modal: "true",
            section {
                class: "h-full w-full max-w-xl overflow-y-auto bg-panel p-6 shadow-xl md:m-4 md:h-auto md:max-h-[calc(100vh-2rem)] md:rounded-lg",
                h2 { class: "text-xl font-semibold text-ink-strong", "Evidence" }
                p { class: "mt-2 font-semibold", "{evidence.artifact_title}" }
                p { class: "mt-4 whitespace-pre-wrap", "{evidence.excerpt}" }
                button {
                    class: "mt-6 rounded bg-accent px-4 py-2 text-white",
                    onclick: move |_| on_close.call(()),
                    "Close"
                }
            }
        }
    }
}
