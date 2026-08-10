use dioxus::prelude::*;

use crate::{api::NotebookSummary, route::Route};

#[component]
pub(crate) fn GlobalNav(notebooks: Vec<NotebookSummary>) -> Element {
    let route = use_route::<crate::route::Route>();
    let active_class = "font-semibold text-accent";
    let idle_class = "text-ink-muted hover:text-ink-strong";
    let search_class = if matches!(&route, Route::Search { .. }) {
        active_class
    } else {
        idle_class
    };
    let retrieval_class = if matches!(&route, Route::Retrieval { .. }) {
        active_class
    } else {
        idle_class
    };
    let tasks_class = if matches!(&route, Route::Tasks { .. }) {
        active_class
    } else {
        idle_class
    };
    let index_class = if matches!(&route, Route::Index { .. }) {
        active_class
    } else {
        idle_class
    };
    let notebooks_class = if matches!(
        &route,
        Route::Dashboard { .. }
            | Route::NotebookOverview { .. }
            | Route::NotebookSources { .. }
            | Route::NotebookAsk { .. }
            | Route::NotebookDrafts { .. }
    ) {
        active_class
    } else {
        idle_class
    };
    rsx! {
        aside { class: "hidden w-60 shrink-0 lg:block", aria_label: "Workspaces",
            nav { class: "mb-6 space-y-1", aria_label: "Global",
                a {
                    class: "block rounded px-3 py-2 {search_class}",
                    href: "/search",
                    "Search"
                }
                a {
                    class: "block rounded px-3 py-2 {notebooks_class}",
                    href: "/",
                    "Notebooks"
                }
                a {
                    class: "block rounded px-3 py-2 {retrieval_class}",
                    href: "/retrieval",
                    "Retrieval"
                }
                a {
                    class: "block rounded px-3 py-2 {tasks_class}",
                    href: "/tasks",
                    "Tasks"
                }
                a {
                    class: "block rounded px-3 py-2 {index_class}",
                    href: "/index",
                    "Index"
                }
            }
            h2 {
                class: "mb-3 text-sm font-semibold uppercase tracking-wide text-ink-muted",
                "Notebooks"
            }
            for notebook in notebooks {
                a {
                    class: "block rounded px-3 py-2 text-ink-muted hover:text-ink-strong",
                    href: "/notebooks/{notebook.notebook_id}",
                    "{notebook.title}"
                }
            }
        }
    }
}

#[component]
pub(crate) fn NotebookSelector(
    notebooks: Vec<NotebookSummary>,
    selected_notebook: String,
) -> Element {
    let navigator = use_navigator();
    rsx! {
        label { class: "sr-only", r#for: "notebook-selector", "Select notebook" }
        select {
            id: "notebook-selector",
            class: "w-full rounded border border-line bg-input px-3 py-2",
            value: selected_notebook,
            onchange: move |event| {
                let value = event.value();
                match value.as_str() {
                    "search" => {
                        navigator.push(Route::Search {});
                    }
                    "retrieval" => {
                        navigator.push(Route::Retrieval {});
                    }
                    "tasks" => {
                        navigator.push(Route::Tasks {});
                    }
                    "index" => {
                        navigator.push(Route::Index {});
                    }
                    _ => {
                        if let Ok(id) = value.parse::<u64>() {
                            navigator.push(Route::NotebookOverview { notebook_id: id });
                        }
                    }
                }
            },
            option { value: "", "Select notebook" }
            option { value: "search", "Search" }
            option { value: "retrieval", "Retrieval" }
            option { value: "tasks", "Tasks" }
            option { value: "index", "Index" }
            for notebook in notebooks {
                option { value: "{notebook.notebook_id}", "{notebook.title}" }
            }
        }
    }
}
