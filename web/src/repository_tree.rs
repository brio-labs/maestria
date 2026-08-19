//! The lazy expandable repository selection tree: directories expand on
//! demand to their classified subdirectories and direct files, so selection
//! can go arbitrarily deep or stay at the top.

use dioxus::prelude::*;

use crate::{
    api::{
        ApiClient, CandidateDirWire, IndexPolicyWire, RepositoryIndexFileWire,
        RepositoryIndexFilesWire,
    },
    components::{alert, class_badge},
    index::Included,
    state::LoadState,
};

/// Repository-relative directory paths currently expanded in the tree.
pub(crate) type Expanded = std::collections::BTreeSet<String>;
/// Per-directory lazy child caches, keyed by repository-relative path.
pub(crate) type ChildrenCache =
    std::collections::BTreeMap<String, LoadState<Vec<CandidateDirWire>>>;
/// Per-directory lazy file caches, keyed by repository-relative path.
pub(crate) type FilesCache =
    std::collections::BTreeMap<String, LoadState<RepositoryIndexFilesWire>>;

/// The repository-relative form of an absolute tree path (leading slash
/// trimmed, absolute fallback for out-of-root paths).
fn relative_of(root: &str, path: &str) -> String {
    match path.strip_prefix(root) {
        Some(relative) => relative.trim_start_matches('/').to_string(),
        None => path.to_string(),
    }
}

/// The top-level tree: one row per direct child of the scanned root.
#[component]
pub(crate) fn BrowseTree(
    api: ApiClient,
    root: String,
    node: CandidateDirWire,
    included: Signal<Included>,
    expanded: Signal<Expanded>,
    children_cache: Signal<ChildrenCache>,
    files_cache: Signal<FilesCache>,
) -> Element {
    rsx! {
        ul { class: "space-y-1", role: "list",
            for child in node.children.iter() {
                BrowseRow {
                    api: api.clone(),
                    root: root.clone(),
                    node: child.clone(),
                    included,
                    expanded,
                    children_cache,
                    files_cache
                }
            }
        }
    }
}

/// One directory row: selection checkbox, class badge, expand toggle, and
/// the expanded content (subdirectories and direct files) when open.
#[component]
fn BrowseRow(
    api: ApiClient,
    root: String,
    node: CandidateDirWire,
    included: Signal<Included>,
    expanded: Signal<Expanded>,
    children_cache: Signal<ChildrenCache>,
    files_cache: Signal<FilesCache>,
) -> Element {
    let path = node.path.clone();
    let relative = relative_of(&root, &node.path);
    let class = node.class.clone();
    let checked = included.read().contains_key(&node.path);
    let size_mb = node.total_bytes as f64 / (1024.0 * 1024.0);
    let badge = class_badge(&class);
    let direct_files = node.file_count
        > node
            .children
            .iter()
            .map(|child| child.file_count)
            .sum::<usize>();
    let expandable = !node.children.is_empty() || direct_files;
    let is_expanded = expanded.read().contains(&relative);
    let toggle_path = node.path.clone();
    let default_policy = node.policy.clone();
    // A single-file leaf group (a direct file of the scanned root, e.g.
    // `Cargo.toml`) is a file, not a directory: no expansion, no policies.
    let file_leaf = node.children.is_empty()
        && node
            .path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.contains('.'));
    if file_leaf {
        return rsx! {
            FileLeafRow { node_path: node.path.clone(), class, checked, included, default_policy }
        };
    }
    rsx! {
        li { class: "rounded-lg border border-line bg-panel p-3",
            div { class: "flex flex-wrap items-center justify-between gap-3",
                div { class: "flex min-w-0 items-center gap-2",
                    if expandable {
                        ExpandToggle {
                            api: api.clone(),
                            root: root.clone(),
                            relative: relative.clone(),
                            expanded,
                            children_cache,
                            files_cache,
                            is_expanded,
                            node_path: node.path.clone()
                        }
                    } else {
                        span { class: "w-5" }
                    }
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
                        span {
                            class: "rounded px-2 py-1 text-xs text-white {badge}",
                            {class}
                        }
                        span { class: "text-xs text-ink-muted",
                            "{node.file_count} files · {size_mb:.1} MB"
                        }
                    }
                }
            }
            if checked {
                crate::index::PolicyToggles { path: path.clone(), included }
            }
            if is_expanded {
                ExpandedContent {
                    api: api.clone(),
                    root: root.clone(),
                    relative,
                    included,
                    expanded,
                    children_cache,
                    files_cache
                }
            }
        }
    }
}

/// A file rendered directly from the candidate tree (a single-file leaf
/// group, e.g. a manifest at the scanned root): selectable, classified,
/// never expandable.
#[component]
fn FileLeafRow(
    node_path: String,
    class: String,
    checked: bool,
    included: Signal<Included>,
    default_policy: IndexPolicyWire,
) -> Element {
    let toggle_path = node_path.clone();
    let badge = class_badge(&class);
    rsx! {
        li { class: "rounded-lg border border-line bg-panel px-3 py-1.5",
            label { class: "flex min-w-0 items-center gap-2",
                input {
                    r#type: "checkbox",
                    aria_label: "Include {node_path}",
                    checked: checked,
                    onchange: move |event| {
                        let mut map = included.write();
                        if event.value() == "true" {
                            map.insert(toggle_path.clone(), default_policy.clone());
                        } else {
                            map.remove(&toggle_path);
                        }
                    }
                }
                span { class: "truncate font-mono text-xs", {node_path.clone()} }
                span {
                    class: "rounded px-1.5 py-0.5 text-[10px] text-white {badge}",
                    {class}
                }
            }
        }
    }
}

/// The expand/collapse toggle for one directory. First expansion fetches
/// the classified subdirectories and the direct file listing on demand.
#[component]
fn ExpandToggle(
    api: ApiClient,
    root: String,
    relative: String,
    expanded: Signal<Expanded>,
    children_cache: Signal<ChildrenCache>,
    files_cache: Signal<FilesCache>,
    is_expanded: bool,
    node_path: String,
) -> Element {
    let expand_rel = relative.clone();
    let expand_root = root.clone();
    let expand_api = api.clone();
    rsx! {
        button {
            class: "rounded px-1.5 text-ink-muted hover:text-ink-strong",
            aria_label: "Expand {node_path}",
            onclick: move |_| {
                let mut set = expanded.write();
                if !set.insert(expand_rel.clone()) {
                    set.remove(&expand_rel);
                    return;
                }
                if children_cache.read().contains_key(&expand_rel)
                    || files_cache.read().contains_key(&expand_rel)
                {
                    return;
                }
                let api = expand_api.clone();
                let root = expand_root.clone();
                let rel = expand_rel.clone();
                spawn(async move {
                    let children = api.repository_index_children(&root, &rel).await;
                    match children {
                        Ok(children) => children_cache
                            .write()
                            .insert(rel.clone(), LoadState::Ready(children)),
                        Err(error) => children_cache
                            .write()
                            .insert(rel.clone(), LoadState::Failed(error)),
                    };
                    let files = api.repository_index_files(&root, &rel).await;
                    match files {
                        Ok(files) => files_cache
                            .write()
                            .insert(rel.clone(), LoadState::Ready(files)),
                        Err(error) => files_cache
                            .write()
                            .insert(rel.clone(), LoadState::Failed(error)),
                    };
                });
            },
            {if is_expanded { "▾" } else { "▸" }}
        }
    }
}

/// The expanded section of a directory: its classified subdirectories
/// (recursively expandable) and its direct files. The two fetches share a
/// single loading indicator that names the pending part once the other has
/// landed, so an expansion never shows duplicate "Loading…" lines.
#[component]
fn ExpandedContent(
    api: ApiClient,
    root: String,
    relative: String,
    included: Signal<Included>,
    expanded: Signal<Expanded>,
    children_cache: Signal<ChildrenCache>,
    files_cache: Signal<FilesCache>,
) -> Element {
    let children_state = children_cache.read().get(&relative).cloned();
    let files_state = files_cache.read().get(&relative).cloned();
    let children_ready = matches!(children_state, Some(LoadState::Ready(_)));
    let files_ready = matches!(files_state, Some(LoadState::Ready(_)));
    let pending_hint = if children_ready && !files_ready {
        "Loading files…"
    } else if files_ready && !children_ready {
        "Loading subdirectories…"
    } else {
        "Loading…"
    };
    rsx! {
        div { class: "ml-6 mt-2 border-l border-line pl-3",
            if !children_ready || !files_ready {
                p { class: "text-xs text-ink-muted", {pending_hint} }
            }
            if let Some(LoadState::Failed(error)) = &children_state {
                {alert(error)}
            }
            if let Some(LoadState::Failed(error)) = &files_state {
                {alert(error)}
            }
            if children_ready {
                if let Some(LoadState::Ready(children)) = &children_state {
                    for child in children.iter() {
                        BrowseRow {
                            api: api.clone(),
                            root: root.clone(),
                            node: child.clone(),
                            included,
                            expanded,
                            children_cache,
                            files_cache
                        }
                    }
                }
            }
            if files_ready {
                if let Some(LoadState::Ready(files)) = &files_state {
                    for file in files.files.iter() {
                        FileRow { root: root.clone(), file: file.clone(), included }
                    }
                    if files.truncated {
                        p { class: "text-xs text-ink-muted", "… and more files (listing capped)" }
                    }
                }
            }
        }
    }
}

/// One direct file of an expanded directory. Files covered by a selected
/// ancestor directory are shown included and cannot be unchecked there;
/// uncheck the directory to select files individually. A file selected
/// itself stays checkable so it can be deselected.
#[component]
fn FileRow(root: String, file: RepositoryIndexFileWire, included: Signal<Included>) -> Element {
    let path = file.path.clone();
    let covered = included.read().keys().any(|key| {
        let key_relative = match key.strip_prefix(&root) {
            Some(relative) => relative.trim_start_matches('/'),
            None => key.as_str(),
        };
        !key_relative.is_empty()
            && key_relative != path
            && path.starts_with(&format!("{key_relative}/"))
    });
    let checked = covered;
    let kind = file.kind.clone();
    let kind_badge = match kind.as_str() {
        "code" => "bg-accent",
        "doc" => "bg-success",
        "manifest" => "bg-warning",
        _ => "bg-muted",
    };
    rsx! {
        li { class: "rounded-lg border border-line bg-panel px-3 py-1.5",
            label { class: "flex min-w-0 items-center gap-2",
                input {
                    r#type: "checkbox",
                    aria_label: "Include {file.path}",
                    checked: checked,
                    disabled: covered,
                    onchange: move |event| {
                        let mut map = included.write();
                        if event.value() == "true" {
                            map.insert(path.clone(), IndexPolicyWire::default());
                        } else {
                            map.remove(&path);
                        }
                    }
                }
                span { class: "truncate font-mono text-xs", "{file.path}" }
                span { class: "rounded px-1.5 py-0.5 text-[10px] text-white {kind_badge}", {kind} }
                span { class: "text-[10px] text-ink-muted", {human_size(file.size)} }
            }
        }
    }
}

/// Compact human-readable size for file rows.
fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
