use dioxus::prelude::*;

use crate::{
    api::{ApiClient, CandidateDirWire, IndexCandidatesWire, IndexPolicyWire, IndexRunWire},
    components::{Shell, WorkspaceContext, alert},
    route::Route,
    state::LoadState,
};

/// The default "skip large files" ceiling, matching the choice layer.
const DEFAULT_MAX_FILE_BYTES: u64 = 1024 * 1024;

/// Included directories: path → the policy its files run under. A
/// directory is whitelisted exactly when it is present in this map.
pub(crate) type Included = std::collections::BTreeMap<String, IndexPolicyWire>;

/// The tab bar shared by the two indexing subsections: document indexing
/// (`/index`) and repository code indexing (`/index/repositories`).
#[component]
pub(crate) fn IndexTabs() -> Element {
    let route = use_route::<Route>();
    let documents_active = matches!(&route, Route::Index { .. });
    let repositories_active = matches!(&route, Route::IndexRepositories { .. });
    let tab_class = |active: bool| {
        if active {
            "rounded-t border-b-2 border-accent px-4 py-2 font-semibold text-accent"
        } else {
            "rounded-t px-4 py-2 text-ink-muted hover:text-ink-strong"
        }
    };
    rsx! {
        div { class: "mb-6 flex gap-1 border-b border-line",
            a {
                class: tab_class(documents_active),
                href: "/index",
                aria_current: if documents_active { "page" } else { "false" },
                "Documents"
            }
            a {
                class: tab_class(repositories_active),
                href: "/index/repositories",
                aria_current: if repositories_active { "page" } else { "false" },
                "Repositories"
            }
        }
    }
}

#[component]
pub(crate) fn IndexWorkspace() -> Element {
    let context = use_context::<Signal<WorkspaceContext>>();
    let api = use_hook(ApiClient::new);
    let root = use_signal(|| match context.read().bootstrap_status.as_ref() {
        Some(status) if !status.instance_root_path.is_empty() => status.instance_root_path.clone(),
        Some(status) if status.instance_root.starts_with('/') => status.instance_root.clone(),
        _ => String::new(),
    });
    let candidates: Signal<LoadState<IndexCandidatesWire>> = use_signal(|| LoadState::Empty);
    let included: Signal<Included> = use_signal(Included::default);
    let run_result = use_signal(|| None::<IndexRunWire>);
    let running = use_signal(|| false);

    rsx! {
        Shell { title: "Index", active_notebook: None,
            IndexTabs {}
            p { class: "mb-6 text-ink-muted",
                "Choose what gets indexed: the whitelist decides, and per-directory policy "
                "switches filter large, generated, or minified files."
            }
            ScanForm { api: api.clone(), root, candidates, included, run_result }
            match candidates() {
                LoadState::Loading => rsx! { p { "Scanning candidates…" } },
                LoadState::Failed(error) => rsx! { {alert(&error)} },
                LoadState::Empty => rsx! { p { class: "text-ink-muted", "No scan yet." } },
                LoadState::Ready(response) => rsx! {
                    CandidateTree { node: response.tree.clone(), included }
                    RunControls {
                        api: api.clone(),
                        response: response.clone(),
                        candidates,
                        included,
                        run_result,
                        running
                    }
                },
            }
        }
    }
}

#[component]
fn ScanForm(
    api: ApiClient,
    mut root: Signal<String>,
    mut candidates: Signal<LoadState<IndexCandidatesWire>>,
    mut included: Signal<Included>,
    mut run_result: Signal<Option<IndexRunWire>>,
) -> Element {
    let scanning = matches!(candidates(), LoadState::Loading);
    rsx! {
        form {
            class: "mb-6 flex flex-col gap-3 sm:flex-row",
            onsubmit: move |event| {
                event.prevent_default();
                let text = root.read().trim().to_owned();
                if text.is_empty() {
                    return;
                }
                candidates.set(LoadState::Loading);
                run_result.set(None);
                let api = api.clone();
                spawn(async move {
                    match api.index_candidates(&text).await {
                        Ok(response) => {
                            let mut selected = Included::new();
                            for child in &response.tree.children {
                                collect_recommended(child, &mut selected);
                            }
                            included.set(selected);
                            candidates.set(LoadState::Ready(response));
                        }
                        Err(error) => candidates.set(LoadState::Failed(error)),
                    }
                });
            },
            label { class: "sr-only", r#for: "index-root", "Root path" }
            input {
                id: "index-root",
                class: "flex-1 rounded border border-line bg-input px-3 py-2",
                placeholder: "Absolute path to scan, e.g. /home/you/notes",
                value: "{root}",
                oninput: move |event| root.set(event.value())
            }
            button {
                class: "rounded bg-accent px-4 py-2 font-semibold text-white disabled:opacity-50",
                disabled: scanning || root.read().trim().is_empty(),
                {if scanning { "Scanning…" } else { "Scan" }}
            }
        }
    }
}

#[component]
fn RunControls(
    api: ApiClient,
    response: IndexCandidatesWire,
    mut candidates: Signal<LoadState<IndexCandidatesWire>>,
    included: Signal<Included>,
    mut run_result: Signal<Option<IndexRunWire>>,
    mut running: Signal<bool>,
) -> Element {
    rsx! {
        div { class: "mt-6 flex items-center gap-4",
            button {
                class: "rounded bg-accent px-4 py-2 font-semibold text-white disabled:opacity-50",
                disabled: running() || included.read().is_empty(),
                onclick: move |_| {
                    run_result.set(None);
                    running.set(true);
                    let api = api.clone();
                    let root = response.root.clone();
                    let includes: Vec<String> = included.read().keys().cloned().collect();
                    let policies = included.read().clone();
                    spawn(async move {
                        let outcome = api.index_run(&root, includes, policies).await;
                        running.set(false);
                        match outcome {
                            Ok(result) => run_result.set(Some(result)),
                            Err(error) => candidates.set(LoadState::Failed(error)),
                        }
                    });
                },
                {if running() { "Indexing…" } else { "Index selected" }}
            }
            if let Some(result) = run_result() {
                span { role: "status", class: "text-ink-strong",
                    "submitted {result.submitted} · skipped {result.skipped}"
                }
            }
        }
    }
}

/// Collect the default whitelist: every `Recommended` directory under
/// `node` with the policy the classifier assigned.
pub(crate) fn collect_recommended(node: &CandidateDirWire, selected: &mut Included) {
    if node.class == "Recommended" {
        selected.insert(node.path.clone(), node.policy.clone());
        return;
    }
    for child in &node.children {
        collect_recommended(child, selected);
    }
}

#[component]
pub(crate) fn CandidateTree(node: CandidateDirWire, included: Signal<Included>) -> Element {
    rsx! {
        ul { class: "space-y-1", role: "list",
            for child in node.children.iter() {
                CandidateRow { node: child.clone(), included }
            }
        }
    }
}

#[component]
pub(crate) fn CandidateRow(node: CandidateDirWire, included: Signal<Included>) -> Element {
    let path = node.path.clone();
    let class = node.class.clone();
    let checked = included.read().contains_key(&node.path);
    let size_mb = node.total_bytes as f64 / (1024.0 * 1024.0);
    let class_badge = match class.as_str() {
        "Recommended" => "bg-success",
        "Noise" => "bg-muted",
        _ => "bg-warning",
    };
    let toggle_path = path.clone();
    let default_policy = node.policy.clone();
    rsx! {
        li { class: "rounded-lg border border-line bg-panel p-3",
            div { class: "flex flex-wrap items-center justify-between gap-3",
                label { class: "flex min-w-0 items-center gap-3",
                    input {
                        r#type: "checkbox",
                        aria_label: "Include {node.path}",
                        checked: checked,
                        onchange: move |event| {
                            let mut map = included.write();
                            if event.value() == "true" {
                                map.entry(toggle_path.clone())
                                    .or_insert_with(|| default_policy.clone());
                            } else {
                                map.remove(&toggle_path);
                            }
                        }
                    }
                    span { class: "truncate font-mono text-sm", "{node.path}" }
                    span { class: "rounded px-2 py-1 text-xs text-white {class_badge}", {class} }
                    span { class: "text-xs text-ink-muted",
                        "{node.file_count} files · {size_mb:.1} MB"
                    }
                }
            }
            if included.read().contains_key(&node.path) {
                PolicyToggles { path: path.clone(), included }
            }
            if !node.children.is_empty() {
                div { class: "ml-6 mt-2 border-l border-line pl-3",
                    for child in node.children.iter() {
                        CandidateRow { node: child.clone(), included }
                    }
                }
            }
        }
    }
}

#[component]
pub(crate) fn PolicyToggles(path: String, included: Signal<Included>) -> Element {
    let policy = match included.read().get(&path) {
        Some(policy) => policy.clone(),
        None => IndexPolicyWire {
            max_file_bytes: 0,
            skip_generated: false,
            skip_minified: false,
        },
    };
    let max_file_bytes = policy.max_file_bytes;
    let skip_generated = policy.skip_generated;
    let skip_minified = policy.skip_minified;
    let large_path = path.clone();
    let generated_path = path.clone();
    let minified_path = path.clone();
    rsx! {
        div { class: "mt-2 flex flex-wrap gap-4 pl-8 text-sm text-ink-muted",
            label { class: "flex items-center gap-2",
                input {
                    r#type: "checkbox",
                    checked: max_file_bytes > 0,
                    onchange: move |event| {
                        let mut map = included.write();
                        if let Some(policy) = map.get_mut(&large_path) {
                            policy.max_file_bytes = if event.value() == "true" {
                                DEFAULT_MAX_FILE_BYTES
                            } else {
                                0
                            };
                        }
                    }
                }
                "Skip large files"
            }
            label { class: "flex items-center gap-2",
                input {
                    r#type: "checkbox",
                    checked: skip_generated,
                    onchange: move |event| {
                        let mut map = included.write();
                        if let Some(policy) = map.get_mut(&generated_path) {
                            policy.skip_generated = event.value() == "true";
                        }
                    }
                }
                "Skip generated dumps"
            }
            label { class: "flex items-center gap-2",
                input {
                    r#type: "checkbox",
                    checked: skip_minified,
                    onchange: move |event| {
                        let mut map = included.write();
                        if let Some(policy) = map.get_mut(&minified_path) {
                            policy.skip_minified = event.value() == "true";
                        }
                    }
                }
                "Skip minified bundles"
            }
        }
    }
}
