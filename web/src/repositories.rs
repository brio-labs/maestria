use dioxus::prelude::*;

use crate::{
    api::{
        ApiClient, RepositoryIndexCandidatesWire, RepositoryIndexProgressWire,
        RepositoryIndexRunWire, RepositoryIndexStatusWire,
    },
    components::{Shell, WorkspaceContext, alert},
    index::collect_recommended,
    repository_tree::{BrowseTree, ChildrenCache, Expanded, FilesCache},
    state::LoadState,
};

/// Included directories: path → the policy its files run under. A
/// directory is whitelisted exactly when it is present in this map.
type Included = crate::index::Included;

#[component]
pub(crate) fn RepositoryIndexWorkspace() -> Element {
    let context = use_context::<Signal<WorkspaceContext>>();
    let api = use_hook(ApiClient::new);
    let root = use_signal(|| match context.read().bootstrap_status.as_ref() {
        Some(status) if status.instance_root.starts_with('/') => status.instance_root.clone(),
        _ => String::new(),
    });
    let candidates: Signal<LoadState<RepositoryIndexCandidatesWire>> =
        use_signal(|| LoadState::Empty);
    let included: Signal<Included> = use_signal(Included::default);
    let run_result = use_signal(|| None::<RepositoryIndexRunWire>);
    let status: Signal<LoadState<RepositoryIndexStatusWire>> = use_signal(|| LoadState::Empty);
    let running = use_signal(|| false);
    let expanded: Signal<Expanded> = use_signal(Expanded::default);
    let children_cache: Signal<ChildrenCache> = use_signal(ChildrenCache::default);
    let files_cache: Signal<FilesCache> = use_signal(FilesCache::default);
    let live_progress: Signal<Option<RepositoryIndexProgressWire>> = use_signal(|| None);

    rsx! {
        Shell { title: "Repositories", active_notebook: None,
            crate::index::IndexTabs {}
            p { class: "mb-6 text-ink-muted",
                "Choose which repository directories get code-indexed: the selection decides, "
                "per-directory policy filters large or minified files, and identity, delta, "
                "records, and freshness are scoped to the selection."
            }
            ScanForm {
                api: api.clone(),
                root,
                candidates,
                included,
                run_result,
                status,
                live_progress
            }
            match candidates() {
                LoadState::Loading => rsx! { p { "Scanning candidates…" } },
                LoadState::Failed(error) => rsx! { {alert(&error)} },
                LoadState::Empty => rsx! { p { class: "text-ink-muted", "No scan yet." } },
                LoadState::Ready(response) => rsx! {
                    BrowseTree {
                        api: api.clone(),
                        root: response.root.clone(),
                        node: response.tree.clone(),
                        included,
                        expanded,
                        children_cache,
                        files_cache
                    }
                    RunControls {
                        api: api.clone(),
                        response: response.clone(),
                        candidates,
                        included,
                        run_result,
                        status,
                        running,
                        live_progress
                    }
                },
            }
            StatusPanel { status, root, live_progress }
        }
    }
}

#[component]
fn ScanForm(
    api: ApiClient,
    mut root: Signal<String>,
    mut candidates: Signal<LoadState<RepositoryIndexCandidatesWire>>,
    mut included: Signal<Included>,
    mut run_result: Signal<Option<RepositoryIndexRunWire>>,
    mut status: Signal<LoadState<RepositoryIndexStatusWire>>,
    mut live_progress: Signal<Option<RepositoryIndexProgressWire>>,
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
                live_progress.set(None);
                let api = api.clone();
                spawn(async move {
                    match api.repository_index_candidates(&text).await {
                        Ok(response) => {
                            let mut selected = Included::new();
                            for child in &response.tree.children {
                                collect_recommended(child, &mut selected);
                            }
                            included.set(selected);
                            candidates.set(LoadState::Ready(response));
                            match api.repository_index_status(&text).await {
                                Ok(wire) => status.set(LoadState::Ready(wire)),
                                Err(error) => status.set(LoadState::Failed(error)),
                            }
                        }
                        Err(error) => candidates.set(LoadState::Failed(error)),
                    }
                });
            },
            label { class: "sr-only", r#for: "repository-index-root", "Repository root" }
            input {
                id: "repository-index-root",
                class: "flex-1 rounded border border-line bg-input px-3 py-2",
                placeholder: "Absolute repository path, e.g. /home/you/projects/maestria",
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
    response: RepositoryIndexCandidatesWire,
    mut candidates: Signal<LoadState<RepositoryIndexCandidatesWire>>,
    included: Signal<Included>,
    mut run_result: Signal<Option<RepositoryIndexRunWire>>,
    mut status: Signal<LoadState<RepositoryIndexStatusWire>>,
    mut running: Signal<bool>,
    live_progress: Signal<Option<RepositoryIndexProgressWire>>,
) -> Element {
    rsx! {
        div { class: "mt-6 flex items-center gap-4",
            button {
                class: "rounded bg-accent px-4 py-2 font-semibold text-white disabled:opacity-50",
                disabled: running() || included.read().is_empty(),
                onclick: move |_| {
                    run_result.set(None);
                    live_progress.set(None);
                    running.set(true);
                    // Poll the lightweight progress endpoint while the run
                    // is in flight so the status panel shows live progress.
                    let poll_api = api.clone();
                    let mut poll_live = live_progress;
                    let poll_running = running;
                    spawn(async move {
                        while poll_running() {
                            if let Ok(progress) = poll_api.repository_index_progress().await {
                                poll_live.set(progress);
                            }
                            gloo_timers::future::TimeoutFuture::new(3000).await;
                        }
                    });
                    let api = api.clone();
                    let root = response.root.clone();
                    let includes: Vec<String> = included.read().keys().cloned().collect();
                    let policies = included.read().clone();
                    spawn(async move {
                        let outcome = api.repository_index_run(&root, includes, policies).await;
                        running.set(false);
                        live_progress.set(None);
                        match outcome {
                            Ok(result) => {
                                run_result.set(Some(result.clone()));
                                match api.repository_index_status(&root).await {
                                    Ok(wire) => status.set(LoadState::Ready(wire)),
                                    Err(error) => status.set(LoadState::Failed(error)),
                                }
                            }
                            Err(error) => candidates.set(LoadState::Failed(error)),
                        }
                    });
                },
                {if running() { "Indexing…" } else { "Index selected" }}
            }
            if let Some(result) = run_result() {
                span { role: "status", class: "text-ink-strong",
                    "mode={result.mode} · {result.summary.symbol_count} symbols · "
                    "{result.summary.file_count} files · registered {result.registered} · "
                    "skipped {result.skipped}"
                }
            }
        }
    }
}

/// The persisted repository code index status: present/freshness, counts,
/// and the recorded selection (auditable scope).
/// The human-readable progress line for a run progress snapshot.
fn progress_text(progress: &RepositoryIndexProgressWire) -> Option<String> {
    if progress.phase == "building" {
        Some("building index…".to_string())
    } else if progress.total > 0 {
        Some(format!(
            "registering {}/{} sources",
            progress.registered, progress.total
        ))
    } else {
        None
    }
}

#[component]
fn StatusPanel(
    status: Signal<LoadState<RepositoryIndexStatusWire>>,
    root: Signal<String>,
    live_progress: Signal<Option<RepositoryIndexProgressWire>>,
) -> Element {
    let panel_root = root.read().trim().to_owned();
    rsx! {
        div { class: "mt-8 rounded-lg border border-line bg-panel p-4",
            h2 { class: "mb-3 text-sm font-semibold uppercase tracking-wide text-ink-muted",
                "Index status"
            }
            if panel_root.is_empty() {
                p { class: "text-sm text-ink-muted", "Scan a repository to load its status." }
            } else {
                match status() {
                    LoadState::Loading => rsx! {
                        p { class: "text-sm text-ink-muted", "Loading…" }
                    },
                    LoadState::Failed(error) => rsx! { {alert(&error)} },
                    LoadState::Empty => rsx! {
                        p { class: "text-sm text-ink-muted", "No status yet." }
                    },
                    LoadState::Ready(status) => {
                        if status.present {
                            let summary = status.summary.as_ref();
                            let freshness = match status.freshness.as_ref() {
                                Some(crate::api::RepositoryFreshnessWire::Current { .. }) => {
                                    "current"
                                }
                                Some(crate::api::RepositoryFreshnessWire::Stale { .. }) => "stale",
                                None => "unknown",
                            };
                            let selected = match summary {
                                Some(summary) => {
                                    if summary.selected_paths.is_empty() {
                                        "whole-repo".to_string()
                                    } else {
                                        summary.selected_paths.join(",")
                                    }
                                }
                                None => "unknown".to_string(),
                            };
                            let counts = summary.map(|summary| {
                                format!(
                                    "{} packages · {} symbols · {} files",
                                    summary.package_count,
                                    summary.symbol_count,
                                    summary.file_count
                                )
                            });
                            let progress = match live_progress() {
                                Some(progress) => progress_text(&progress),
                                None => status
                                    .progress
                                    .as_ref()
                                    .and_then(progress_text),
                            };
                            rsx! {
                                div { class: "flex flex-wrap gap-4 text-sm",
                                    span { class: "text-ink-strong", "present · {freshness}" }
                                    if let Some(progress) = progress {
                                        span { role: "status", class: "text-accent", {progress} }
                                    }
                                    if let Some(counts) = counts {
                                        span { class: "text-ink-muted", {counts} }
                                    }
                                    span { class: "text-ink-muted", "selected: {selected}" }
                                }
                            }
                        } else {
                            rsx! { p { class: "text-sm text-ink-muted",
                                "No repository code index yet for this root."
                            } }
                        }
                    }
                }
            }
        }
    }
}
