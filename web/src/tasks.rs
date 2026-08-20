use dioxus::prelude::*;

use crate::{
    api::{ApiClient, TaskSummaryWire},
    components::{Shell, alert},
    state::LoadState,
};

#[component]
pub(crate) fn TasksWorkspace() -> Element {
    let api = use_hook(ApiClient::new);
    let mut tasks: Signal<LoadState<Vec<TaskSummaryWire>>> = use_signal(LoadState::default);
    use_effect(move || {
        let api = api.clone();
        spawn(async move {
            match api.tasks().await {
                Ok(value) => {
                    tasks.set(LoadState::ready_or_empty(value));
                }
                Err(error) => tasks.set(LoadState::Failed(error)),
            }
        });
    });
    rsx! {
        Shell { title: "Tasks", active_notebook: None,
            match tasks() {
                LoadState::Loading => rsx! { p { "Loading tasks…" } },
                LoadState::Failed(error) => rsx! { {alert(&error)} },
                LoadState::Empty => rsx! {
                    p { class: "text-ink-muted", "No tasks. Create one with `maestria task start`." }
                },
                LoadState::Ready(items) => rsx! {
                    div { class: "space-y-3",
                        for task in items {
                            TaskCard { task }
                        }
                    }
                },
            }
        }
    }
}

#[component]
fn TaskCard(task: TaskSummaryWire) -> Element {
    let status_class = match task.status.as_str() {
        "Succeeded" | "Completed" => "bg-success",
        "Failed" => "bg-danger",
        _ => "bg-warning",
    };
    rsx! {
        article { class: "rounded-lg border border-line bg-panel p-4",
            div { class: "flex flex-wrap items-center justify-between gap-2",
                h3 { class: "font-semibold text-ink-strong", "{task.title}" }
                div { class: "flex items-center gap-2",
                    span { class: "rounded px-2 py-1 text-xs text-white {status_class}", "{task.status}" }
                    span { class: "rounded bg-muted px-2 py-1 text-xs text-ink-muted", "{task.priority}" }
                    if task.validation_report_id.is_some() {
                        span { class: "rounded bg-accent-soft px-2 py-1 text-xs text-accent", "Validated" }
                    }
                }
            }
            p { class: "mt-2 text-sm text-ink-muted", "{task.evidence_ids.len()} evidence" }
        }
    }
}
