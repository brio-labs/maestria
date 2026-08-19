use dioxus::prelude::*;

use crate::{
    api::{ApiClient, BootstrapStatus, RetrievalRecord, RetrievalStatus},
    components::{Shell, WorkspaceContext, alert},
    state::LoadState,
};

#[component]
pub(crate) fn RetrievalWorkspace() -> Element {
    let context = use_context::<Signal<WorkspaceContext>>();
    let api = use_hook(ApiClient::new);
    let mut status: Signal<LoadState<RetrievalStatus>> = use_signal(LoadState::default);
    use_effect(move || {
        let api = api.clone();
        spawn(async move {
            match api.retrieval().await {
                Ok(value) => status.set(LoadState::Ready(value)),
                Err(error) => status.set(LoadState::Failed(error)),
            }
        });
    });
    rsx! {
        Shell { title: "Retrieval", active_notebook: None,
            match status() {
                LoadState::Loading => rsx! { p { "Loading retrieval status…" } },
                LoadState::Failed(error) => rsx! { {alert(&error)} },
                LoadState::Ready(value) => rsx! {
                    InstanceStats {
                        status: value.clone(),
                        bootstrap: context.read().bootstrap_status.clone()
                    }
                    LaneTable { lanes: value.lanes.clone() }
                    PromotionRecords { records: value.promotion_records.clone() }
                },
                LoadState::Empty => rsx! { p { "No retrieval status." } },
            }
        }
    }
}

#[component]
fn InstanceStats(status: RetrievalStatus, bootstrap: Option<BootstrapStatus>) -> Element {
    let root = bootstrap
        .as_ref()
        .map_or_else(|| "unknown".to_owned(), |value| value.instance_root.clone());
    let events = bootstrap.as_ref().map_or(0, |value| value.event_count);
    let tasks = bootstrap.as_ref().map_or(0, |value| value.task_count);
    rsx! {
        section { class: "mb-6 rounded-lg border border-line bg-panel p-4",
            h2 { class: "font-semibold text-ink-strong", "Instance" }
            dl { class: "mt-3 grid gap-2 text-sm sm:grid-cols-2 lg:grid-cols-3",
                Stat { label: "Instance root", value: root }
                Stat { label: "Events", value: events.to_string() }
                Stat { label: "Tasks", value: tasks.to_string() }
                Stat { label: "Index generation", value: status.index_generation.to_string() }
                Stat { label: "Corpus snapshot", value: status.corpus_snapshot.to_string() }
                Stat { label: "Fingerprint", value: status.fingerprint }
            }
        }
    }
}

#[component]
fn Stat(label: String, value: String) -> Element {
    rsx! {
        div {
            dt { class: "text-ink-muted", {label} }
            dd { class: "font-mono text-ink-strong", {value} }
        }
    }
}

#[component]
fn LaneTable(lanes: crate::api::RetrievalLane) -> Element {
    rsx! {
        section { class: "mb-6 rounded-lg border border-line bg-panel p-4",
            h2 { class: "font-semibold text-ink-strong", "Retrieval lanes" }
            table { class: "mt-3 w-full text-left text-sm",
                thead {
                    tr { class: "text-ink-muted",
                        th { class: "py-2 pr-4 font-medium", "Lane" }
                        th { class: "py-2 pr-4 font-medium", "State" }
                        th { class: "py-2 font-medium", "Details" }
                    }
                }
                tbody {
                    LaneRow { name: "Lexical", state: "Served" }
                    LaneRow {
                        name: "Hybrid",
                        state: lanes.hybrid_state.clone(),
                        details: hybrid_details(&lanes)
                    }
                    LaneRow {
                        name: "Learned sparse",
                        state: lanes.learned_sparse_state.clone(),
                        details: lanes.learned_sparse_model.clone()
                    }
                    LaneRow {
                        name: "Dense",
                        state: if lanes.dense_enabled { "Enabled" } else { "Off" },
                        details: lanes.dense_model.clone()
                    }
                    LaneRow {
                        name: "Repository code",
                        state: lanes.repository_code_state.clone()
                    }
                    LaneRow { name: "Visual", state: lanes.visual_state.clone() }
                }
            }
        }
    }
}

fn hybrid_details(lanes: &crate::api::RetrievalLane) -> Option<String> {
    let mut parts = Vec::new();
    for class in &lanes.hybrid_served_classes {
        parts.push(class.clone());
    }
    if let Some(id) = lanes.hybrid_evaluation_id.as_ref() {
        parts.push(id.clone());
    }
    if let Some(date) = lanes.hybrid_evaluation_date.as_ref() {
        parts.push(date.clone());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

#[component]
fn LaneRow(name: String, state: String, details: Option<String>) -> Element {
    let chip_class = match state.as_str() {
        "Active" | "Served" | "Enabled" => "bg-success",
        "Shadow" | "Off" | "Disabled" => "bg-ink-muted",
        _ => "bg-warning",
    };
    rsx! {
        tr { class: "border-t border-line",
            td { class: "py-2 pr-4 font-medium text-ink-strong", {name} }
            td { class: "py-2 pr-4",
                span { class: "rounded px-2 py-1 text-xs text-white {chip_class}", {state} }
            }
            td { class: "py-2 text-ink-muted",
                if let Some(value) = details {
                    {value}
                }
            }
        }
    }
}

#[component]
fn PromotionRecords(records: crate::api::RetrievalRecords) -> Element {
    rsx! {
        section { class: "rounded-lg border border-line bg-panel p-4",
            h2 { class: "font-semibold text-ink-strong", "Promotion records" }
            div { class: "mt-3 grid gap-4 lg:grid-cols-2",
                RecordCard { lane: "Learned sparse", record: records.learned_sparse.clone() }
                RecordCard { lane: "Hybrid", record: records.hybrid.clone() }
            }
        }
    }
}

#[component]
fn RecordCard(lane: String, record: Option<RetrievalRecord>) -> Element {
    let Some(record) = record else {
        return rsx! {
            div { class: "rounded border border-line bg-muted p-3",
                p { class: "font-medium text-ink-strong", {lane} }
                p { class: "mt-1 text-sm text-ink-muted", "No record — lane serves shadow" }
            }
        };
    };
    rsx! {
        div { class: "rounded border border-line bg-muted p-3",
            p { class: "font-medium text-ink-strong", {lane} }
            dl { class: "mt-2 space-y-1 text-sm",
                RecordField { label: "Evaluation", value: record.evaluation_id }
                RecordField { label: "Corpus", value: record.corpus_id }
                RecordField { label: "Date", value: record.evaluation_date }
                RecordField { label: "Report hash", value: record.report_hash, mono: true }
                RecordField { label: "Created", value: record.created_at }
            }
        }
    }
}

#[component]
fn RecordField(label: String, value: String, #[props(default)] mono: bool) -> Element {
    let class = if mono { "font-mono" } else { "" };
    let title = value.clone();
    rsx! {
        div {
            dt { class: "inline text-ink-muted", {label} }
            dd { class: "inline {class}", title, {value} }
        }
    }
}
