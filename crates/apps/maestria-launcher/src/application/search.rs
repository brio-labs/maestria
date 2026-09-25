use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::{ModelRc, VecModel};

use super::passages::{Passage, PassageSearchResult, ReopenError};
use super::window::show_notice;
use super::{
    AcceptedPassage, AcceptedPath, ActionRow, DisplayedResult, Frontend, LauncherWindow, UiWeak,
    empty_actions, empty_results, lock,
};
use crate::ResultRow;
use crate::errors::LauncherError;
use crate::ipc::LauncherState;
use crate::model::{Action, ResultKind, SearchResponse, SearchResult, SearchStatusKind};

pub(super) fn start_search(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    query: String,
) {
    let previous =
        frontend
            .generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                generation.checked_add(1)
            });
    let generation = match previous {
        Ok(previous) => previous + 1,
        Err(_) => {
            show_notice(
                &ui,
                "Search generation exhausted; restart the launcher.".to_string(),
            );
            return;
        }
    };
    let search_service = state
        .settings()
        .ok()
        .and_then(|settings| settings.search_service());
    {
        let mut model = lock(&frontend.model);
        model.query = query.clone();
        model.pending_ticks = None;
        model.accepted.clear();
        model.accepted_passages.clear();
        model.accepted_paths.clear();
        model.displayed.clear();
        model.content_view_passages.clear();
        model.result_filter = "all".to_string();
        model.selected_file = None;
    }
    if let Some(window) = ui.upgrade() {
        window.set_results(empty_results());
        window.set_actions(empty_actions());
        window.set_passage_view_results(empty_results());
        window.set_passage_view_open(false);
        window.set_notice("".into());
        window.set_result_filter("all".into());
        window.set_index_status(
            if search_service.is_some() && query.is_empty() {
                "Enter a query to search documents"
            } else if search_service.is_some() {
                "Searching document index…"
            } else {
                "Document search not configured"
            }
            .into(),
        );
        window.set_selected_index(0);
        window.set_selected_action_index(0);
        window.set_actions_open(false);
        window.set_status_kind("loading".into());
        window.set_status_message("Searching…".into());
    }
    runtime.spawn(async move {
        let response = state.search(query.clone(), generation).await;
        let apply_frontend = Arc::clone(&frontend);
        let apply_ui = ui.clone();
        let (applied, wait_for_application) = tokio::sync::oneshot::channel::<()>();
        let _ = slint::invoke_from_event_loop(move || {
            if apply_frontend.generation.load(Ordering::Acquire) == generation
                && let Some(window) = apply_ui.upgrade()
            {
                match response {
                    Ok(response) => apply_search_response(&window, &apply_frontend, response),
                    Err(error) => {
                        let mut model = lock(&apply_frontend.model);
                        model.accepted.clear();
                        model.accepted_passages.clear();
                        model.accepted_paths.clear();
                        model.displayed.clear();
                        drop(model);
                        window.set_results(empty_results());
                        window.set_actions(empty_actions());
                        window.set_status_kind("error".into());
                        window.set_status_message(error.message.into());
                    }
                }
            }
            drop(applied);
        });
        let _ = wait_for_application.await;
        if frontend.generation.load(Ordering::Acquire) != generation {
            return;
        }

        if let Some(config) = search_service
            && !query.is_empty()
        {
            let search_result = super::passages::search(config, &query).await;
            let merge_frontend = Arc::clone(&frontend);
            let merge_ui = ui.clone();
            let merge_query = query.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if merge_frontend.generation.load(Ordering::Acquire) != generation {
                    return;
                }
                if let Some(window) = merge_ui.upgrade() {
                    if let Some(search_result) = search_result {
                        apply_passages(
                            &window,
                            &merge_frontend,
                            generation,
                            &merge_query,
                            search_result,
                        );
                    } else {
                        window.set_index_status("Document search unavailable".into());
                    }
                }
            });
        }
    });
}

fn apply_search_response(window: &LauncherWindow, frontend: &Frontend, response: SearchResponse) {
    let status_kind = match &response.status.kind {
        SearchStatusKind::Loading | SearchStatusKind::Refreshing => "loading",
        SearchStatusKind::Ready => "ready",
        SearchStatusKind::Error | SearchStatusKind::CalculationError => "error",
    };
    let mut message = String::new();
    if let Some(status_message) = &response.status.message {
        message.clone_from(status_message);
    }
    let rows = result_rows(&response.results);
    {
        let mut model = lock(&frontend.model);
        model.accepted.clone_from(&response.results);
        model.accepted_passages.clear();
        model.accepted_paths.clear();
        model.content_view_passages.clear();
        model.result_filter = "all".to_string();
        model.displayed = (0..response.results.len())
            .map(DisplayedResult::Application)
            .collect();
        model.selected_file = None;
    }
    window.set_results(rows);
    window.set_result_filter("all".into());
    window.set_passage_view_open(false);
    window.set_passage_view_results(empty_results());
    window.set_actions(actions_for_result(response.results.first()));
    window.set_selected_index(0);
    window.set_selected_action_index(0);
    window.set_actions_open(false);
    window.set_status_kind(status_kind.into());
    window.set_status_message(message.into());
}

fn apply_passages(
    window: &LauncherWindow,
    frontend: &Frontend,
    generation: u64,
    query: &str,
    search_result: PassageSearchResult,
) {
    let metadata = search_result.metadata.summary();
    let accepted_paths = search_result
        .paths
        .into_iter()
        .enumerate()
        .map(|(index, path)| AcceptedPath {
            result_id: format!("path:{generation}:{index}"),
            path: path.path,
        })
        .collect::<Vec<_>>();
    let accepted_passages = search_result
        .passages
        .into_iter()
        .map(|passage| AcceptedPassage {
            result_id: format!("passage:{generation}:{}", passage.evidence_id),
            passage,
        })
        .collect::<Vec<_>>();
    let (rows, selected_index, status_message) = {
        let mut model = lock(&frontend.model);
        if model.query != query || frontend.generation.load(Ordering::Acquire) != generation {
            return;
        }
        model.accepted_paths = accepted_paths;
        model.accepted_passages = accepted_passages;
        model.content_view_passages.clear();
        let (rows, displayed, document_count) = build_result_rows(&model, query, generation);
        let selected_index = choose_selected_result(&displayed, window.get_selected_index());
        let path_count = model.accepted_paths.len();
        let passage_count = model.accepted_passages.len();
        let visible_path_count = displayed
            .iter()
            .filter(|entry| matches!(entry, DisplayedResult::Path(_)))
            .count();
        let visible_passage_count = displayed
            .iter()
            .filter(|entry| matches!(entry, DisplayedResult::Passage(_)))
            .count();
        let result_count = path_count + passage_count;
        let visible_result_count = visible_path_count + visible_passage_count;
        let status_message = if visible_result_count == 0 && result_count > 0 {
            format!(
                "No {} entries are present in the {result_count} returned file paths and cited passages.",
                filter_label(&model.result_filter)
            )
        } else if result_count > 0 {
            format!(
                "{visible_path_count} file paths · {visible_passage_count} cited passages in {document_count} documents."
            )
        } else {
            "No matching file paths or cited passages.".to_string()
        };
        model.displayed = displayed;
        (rows, selected_index, status_message)
    };

    window.set_passage_view_open(false);
    window.set_passage_view_results(empty_results());
    window.set_index_status(metadata.into());
    window.set_results(ModelRc::new(VecModel::from(rows)));
    window.set_selected_index(selected_index as i32);
    window.set_actions_open(false);
    update_selected_actions(window, frontend, selected_index);
    window.set_status_kind("ready".into());
    window.set_status_message(status_message.into());
}

fn build_result_rows(
    model: &super::FrontendModel,
    query: &str,
    generation: u64,
) -> (Vec<ResultRow>, Vec<DisplayedResult>, usize) {
    let mut rows = model.accepted.iter().map(result_row).collect::<Vec<_>>();
    let mut displayed = (0..model.accepted.len())
        .map(DisplayedResult::Application)
        .collect::<Vec<_>>();
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for (index, accepted) in model.accepted_passages.iter().enumerate() {
        groups
            .entry(accepted.passage.location.document_group())
            .or_default()
            .push(index);
    }
    let document_count = groups.len();
    let filter = model.result_filter.as_str();
    if filter != "passages" {
        for (index, accepted) in model.accepted_paths.iter().enumerate() {
            rows.push(path_result_row(accepted));
            displayed.push(DisplayedResult::Path(index));
        }
    }

    for (group_index, (group, indices)) in groups.into_iter().enumerate() {
        let indices = indices
            .into_iter()
            .filter(|index| match filter {
                "files" => model.accepted_passages[*index].passage.location.is_file(),
                "paths" => model.accepted_passages[*index].passage.location.has_path(),
                _ => true,
            })
            .collect::<Vec<_>>();
        if indices.is_empty() {
            continue;
        }

        if filter == "paths" {
            let first = indices[0];
            let count = indices.len();
            let accepted = &model.accepted_passages[first];
            rows.push(document_row(accepted, &group, count));
            displayed.push(DisplayedResult::Passage(first));
            continue;
        }

        rows.push(ResultRow {
            id: format!("passage-group:{generation}:{group_index}").into(),
            title: group.clone().into(),
            subtitle: format!("{} cited passages", indices.len()).into(),
            kind: "passage_group".into(),
            accessible_name: format!("Passages from {group}").into(),
            excerpt_before: "".into(),
            excerpt_match: "".into(),
            excerpt_after: "".into(),
            content: "".into(),
        });
        displayed.push(DisplayedResult::Group);
        for index in indices {
            let accepted = &model.accepted_passages[index];
            let mut row = passage_row(&accepted.result_id, &group, &accepted.passage, query);
            if filter == "files" {
                row.title = "File passage".into();
                row.kind = "file".into();
                row.accessible_name = format!(
                    "File passage from {group}. {}. {}",
                    accepted.passage.location.citation(),
                    accepted.passage.excerpt
                )
                .into();
            }
            rows.push(row);
            displayed.push(DisplayedResult::Passage(index));
        }
    }
    (rows, displayed, document_count)
}

fn choose_selected_result(displayed: &[DisplayedResult], requested: i32) -> usize {
    let requested = requested.max(0) as usize;
    if displayed.get(requested).is_some_and(|result| {
        matches!(
            result,
            DisplayedResult::Application(_)
                | DisplayedResult::Passage(_)
                | DisplayedResult::Path(_)
        )
    }) {
        return requested;
    }
    for (index, result) in displayed.iter().enumerate() {
        if matches!(
            result,
            DisplayedResult::Application(_)
                | DisplayedResult::Passage(_)
                | DisplayedResult::Path(_)
        ) {
            return index;
        }
    }
    0
}

fn document_row(accepted: &AcceptedPassage, group: &str, passage_count: usize) -> ResultRow {
    let citation = accepted.passage.location.citation();
    ResultRow {
        id: accepted.result_id.clone().into(),
        title: group.into(),
        subtitle: format!("Source path · {passage_count} returned passage excerpts · {citation}")
            .into(),
        kind: "document".into(),
        accessible_name: format!(
            "Source path {group}. {passage_count} returned passage excerpts. {citation}"
        )
        .into(),
        excerpt_before: "".into(),
        excerpt_match: "".into(),
        excerpt_after: "".into(),
        content: "".into(),
    }
}

fn path_result_row(accepted: &AcceptedPath) -> ResultRow {
    let name = std::path::Path::new(&accepted.path)
        .file_name()
        .map_or_else(
            || accepted.path.clone(),
            |name| name.to_string_lossy().into_owned(),
        );
    ResultRow {
        id: accepted.result_id.clone().into(),
        title: name.into(),
        subtitle: accepted.path.clone().into(),
        kind: "path".into(),
        accessible_name: format!("File path match {}", accepted.path).into(),
        excerpt_before: "".into(),
        excerpt_match: "".into(),
        excerpt_after: "".into(),
        content: "".into(),
    }
}

pub(super) fn apply_result_filter(
    window: &LauncherWindow,
    frontend: &Frontend,
    requested_filter: &str,
) {
    let filter = match requested_filter {
        "files" | "paths" | "passages" => requested_filter,
        _ => "all",
    };
    let (rows, selected_index, result_count, status_message) = {
        let mut model = lock(&frontend.model);
        model.result_filter = filter.to_string();
        model.selected_file = None;
        model.content_view_passages.clear();
        let generation = frontend.generation.load(Ordering::Acquire);
        let (rows, displayed, document_count) = build_result_rows(&model, &model.query, generation);
        let selected_index = choose_selected_result(&displayed, 0);
        let path_count = model.accepted_paths.len();
        let passage_count = model.accepted_passages.len();
        let visible_path_count = displayed
            .iter()
            .filter(|entry| matches!(entry, DisplayedResult::Path(_)))
            .count();
        let visible_passage_count = displayed
            .iter()
            .filter(|entry| matches!(entry, DisplayedResult::Passage(_)))
            .count();
        let result_count = path_count + passage_count;
        let visible_result_count = visible_path_count + visible_passage_count;
        model.displayed = displayed;
        let status_message = if visible_result_count == 0 && result_count > 0 {
            format!(
                "No {} entries are present in the authorized search results.",
                filter_label(filter)
            )
        } else if result_count > 0 {
            format!(
                "{visible_path_count} file paths · {visible_passage_count} cited passages in {document_count} documents."
            )
        } else {
            String::new()
        };
        (rows, selected_index, result_count, status_message)
    };

    window.set_result_filter(filter.into());
    window.set_passage_view_open(false);
    window.set_passage_view_results(empty_results());
    window.set_results(ModelRc::new(VecModel::from(rows)));
    window.set_selected_index(selected_index as i32);
    window.set_selected_action_index(0);
    window.set_actions_open(false);
    update_selected_actions(window, frontend, selected_index);
    if result_count > 0 {
        window.set_status_kind("ready".into());
        window.set_status_message(status_message.into());
    }
}

fn filter_label(filter: &str) -> &'static str {
    match filter {
        "files" => "file",
        "paths" => "path",
        "passages" => "passage",
        _ => "matching",
    }
}

pub(super) fn open_passage_view(window: &LauncherWindow, frontend: &Frontend, result_id: &str) {
    let (title, rows, passage_indices) = {
        let mut model = lock(&frontend.model);
        let Some(selected) = model
            .accepted_passages
            .iter()
            .position(|accepted| accepted.result_id == result_id)
            .filter(|index| {
                model.displayed.iter().any(|entry| {
                    matches!(entry, DisplayedResult::Passage(displayed) if displayed == index)
                })
            })
        else {
            return;
        };
        let title = model.accepted_passages[selected]
            .passage
            .location
            .document_group();
        let passage_indices = model
            .accepted_passages
            .iter()
            .enumerate()
            .filter_map(|(index, accepted)| {
                (accepted.passage.location.document_group() == title).then_some(index)
            })
            .collect::<Vec<_>>();
        let count = passage_indices.len();
        let rows = passage_indices
            .iter()
            .enumerate()
            .map(|(offset, index)| {
                detail_passage_row(&model.accepted_passages[*index], offset + 1, count)
            })
            .collect::<Vec<_>>();
        model.content_view_passages = passage_indices.clone();
        (title, rows, passage_indices)
    };
    if passage_indices.is_empty() {
        return;
    }
    window.set_notice("".into());
    window.set_passage_view_title(title.into());
    window.set_passage_view_results(ModelRc::new(VecModel::from(rows)));
    window.set_passage_view_open(true);
    window.set_actions_open(false);
}

fn show_reopened_passage(
    window: &LauncherWindow,
    frontend: &Frontend,
    accepted: &AcceptedPassage,
    citation: &str,
    excerpt: &str,
) {
    let (title, row) = {
        let mut model = lock(&frontend.model);
        let Some(index) = model.accepted_passages.iter().position(|current| {
            current.result_id == accepted.result_id
                && current.passage.evidence_id == accepted.passage.evidence_id
                && current.passage.artifact_version == accepted.passage.artifact_version
        }) else {
            return;
        };
        let current = &model.accepted_passages[index];
        let title = current.passage.location.document_group();
        let row = reopened_passage_row(current, citation, excerpt);
        model.content_view_passages = vec![index];
        (title, row)
    };
    window.set_notice("".into());
    window.set_passage_view_title(title.into());
    window.set_passage_view_results(ModelRc::new(VecModel::from(vec![row])));
    window.set_passage_view_open(true);
    window.set_actions_open(false);
}

fn reopened_passage_row(accepted: &AcceptedPassage, citation: &str, excerpt: &str) -> ResultRow {
    ResultRow {
        id: accepted.result_id.clone().into(),
        title: "Reopened cited passage".into(),
        subtitle: citation.into(),
        kind: "passage".into(),
        accessible_name: format!("Reopened cited passage. {citation}. {excerpt}").into(),
        excerpt_before: "".into(),
        excerpt_match: "".into(),
        excerpt_after: "".into(),
        content: excerpt.into(),
    }
}

fn detail_passage_row(accepted: &AcceptedPassage, number: usize, count: usize) -> ResultRow {
    let citation = accepted.passage.location.citation();
    let excerpt_note = if accepted.passage.truncated {
        " · returned excerpt shortened"
    } else {
        ""
    };
    ResultRow {
        id: accepted.result_id.clone().into(),
        title: format!("Passage {number} of {count}").into(),
        subtitle: format!("{citation}{excerpt_note}").into(),
        kind: "passage".into(),
        accessible_name: format!("Passage {number} of {count}. {citation}{excerpt_note}").into(),
        excerpt_before: "".into(),
        excerpt_match: "".into(),
        excerpt_after: "".into(),
        content: accepted.passage.excerpt.clone().into(),
    }
}

pub(super) fn close_passage_view(window: &LauncherWindow, frontend: &Frontend) {
    lock(&frontend.model).content_view_passages.clear();
    window.set_passage_view_open(false);
    window.set_passage_view_results(empty_results());
    window.invoke_focus_search();
}

pub(super) fn passage_result_is_visible(frontend: &Frontend, result_id: &str) -> bool {
    let model = lock(&frontend.model);
    let Some(index) = model
        .accepted_passages
        .iter()
        .position(|accepted| accepted.result_id == result_id)
    else {
        return false;
    };
    model.content_view_passages.contains(&index)
        || model.displayed.iter().any(
            |entry| matches!(entry, DisplayedResult::Passage(displayed) if *displayed == index),
        )
}

pub(super) fn path_result_is_visible(frontend: &Frontend, result_id: &str) -> bool {
    let model = lock(&frontend.model);
    let Some(index) = model
        .accepted_paths
        .iter()
        .position(|accepted| accepted.result_id == result_id)
    else {
        return false;
    };
    model
        .displayed
        .iter()
        .any(|entry| matches!(entry, DisplayedResult::Path(displayed) if *displayed == index))
}

pub(super) fn activate_path_action(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    result_id: String,
    action_id: String,
) {
    if action_id != "path.copy" || !path_result_is_visible(&frontend, &result_id) {
        show_notice(&ui, "This file path is no longer visible.".to_string());
        return;
    }
    let generation = frontend.generation.load(Ordering::Acquire);
    let selected = {
        let model = lock(&frontend.model);
        model
            .accepted_paths
            .iter()
            .find(|accepted| accepted.result_id == result_id)
            .map(|accepted| (model.query.clone(), accepted.path.clone()))
    };
    let Some((query, path)) = selected else {
        show_notice(
            &ui,
            "This file path belongs to an older search.".to_string(),
        );
        return;
    };
    if let Err(error) = state.begin_passage_action(generation) {
        show_notice(&ui, error.message);
        return;
    }
    let config = state
        .settings()
        .ok()
        .and_then(|settings| settings.search_service());
    let Some(config) = config else {
        state.finish_action();
        show_notice(&ui, "Document search is not configured.".to_string());
        return;
    };

    runtime.spawn(async move {
        let fresh = super::passages::search(config, &query)
            .await
            .is_some_and(|result| result.paths.iter().any(|entry| entry.path == path));
        let failed_delivery_state = Arc::clone(&state);
        if slint::invoke_from_event_loop(move || {
            let still_selected = {
                let model = lock(&frontend.model);
                model.query == query
                    && model
                        .accepted_paths
                        .iter()
                        .any(|entry| entry.result_id == result_id && entry.path == path)
            };
            if frontend.generation.load(Ordering::Acquire) != generation
                || state.ensure_generation(generation).is_err()
                || !still_selected
                || !path_result_is_visible(&frontend, &result_id)
            {
                state.finish_action();
                show_notice(
                    &ui,
                    "This file path belongs to an older search.".to_string(),
                );
                return;
            }
            let outcome = if fresh {
                super::platform::copy_text(&path)
                    .map(|()| "Copied freshly verified file path.".to_string())
            } else {
                Err(LauncherError::file_unavailable(
                    "This file path changed or is no longer authorized.",
                ))
            };
            state.finish_action();
            match outcome {
                Ok(message) => show_notice(&ui, message),
                Err(error) => show_notice(&ui, error.message),
            }
        })
        .is_err()
        {
            failed_delivery_state.finish_action();
        }
    });
}

pub(super) fn update_selected_actions(window: &LauncherWindow, frontend: &Frontend, index: usize) {
    let model = lock(&frontend.model);
    if model.selected_file.is_some() {
        window.set_actions(file_actions());
        window.set_selected_action_index(0);
        return;
    }
    let actions = match model.displayed.get(index) {
        Some(DisplayedResult::Application(result_index)) => {
            actions_for_result(model.accepted.get(*result_index))
        }
        Some(DisplayedResult::Passage(passage_index)) => model
            .accepted_passages
            .get(*passage_index)
            .map_or_else(empty_actions, |_| passage_actions()),
        Some(DisplayedResult::Path(path_index)) => model
            .accepted_paths
            .get(*path_index)
            .map_or_else(empty_actions, |_| path_actions()),
        Some(DisplayedResult::Group) | None => empty_actions(),
    };
    window.set_actions(actions);
    window.set_selected_action_index(0);
}

fn result_rows(results: &[SearchResult]) -> ModelRc<ResultRow> {
    ModelRc::new(VecModel::from(
        results.iter().map(result_row).collect::<Vec<_>>(),
    ))
}

fn result_row(result: &SearchResult) -> ResultRow {
    ResultRow {
        id: result.id.clone().into(),
        title: result.title.clone().into(),
        subtitle: result.subtitle.clone().into(),
        kind: result_kind_label(&result.kind).into(),
        accessible_name: format!("{} {}", result.title, result.subtitle).into(),
        excerpt_before: "".into(),
        excerpt_match: "".into(),
        excerpt_after: "".into(),
        content: "".into(),
    }
}

fn passage_row(result_id: &str, group: &str, passage: &Passage, query: &str) -> ResultRow {
    let (before, matched, after) = excerpt_segments(&passage.excerpt, query);
    let citation = passage.location.citation();
    let subtitle = if passage.truncated {
        format!("{citation} · excerpt shortened")
    } else {
        citation.clone()
    };
    let accessible_name = format!(
        "Passage from {group}. {citation}. {}{}{}",
        before, matched, after
    );
    ResultRow {
        id: result_id.into(),
        title: "Passage".into(),
        subtitle: subtitle.into(),
        kind: "passage".into(),
        accessible_name: accessible_name.into(),
        excerpt_before: before.into(),
        excerpt_match: matched.into(),
        excerpt_after: after.into(),
        content: "".into(),
    }
}

fn excerpt_segments(excerpt: &str, query: &str) -> (String, String, String) {
    let range = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .find_map(|term| {
            let term: String = term.chars().take(80).collect();
            find_casefolded_range(excerpt, &term)
        });
    let Some((start, end)) = range else {
        let end = excerpt
            .char_indices()
            .nth(240)
            .map_or(excerpt.len(), |(index, character)| {
                index + character.len_utf8()
            });
        let mut text = excerpt[..end].to_string();
        if end < excerpt.len() {
            text.push('…');
        }
        return (text, String::new(), String::new());
    };

    let context_start = excerpt[..start]
        .char_indices()
        .rev()
        .nth(95)
        .map_or(0, |(index, _)| index);
    let context_end = excerpt[end..]
        .char_indices()
        .nth(160)
        .map_or(excerpt.len(), |(index, character)| {
            end + index + character.len_utf8()
        });
    let mut before = excerpt[context_start..start].to_string();
    if context_start > 0 {
        before.insert(0, '…');
    }
    let matched = excerpt[start..end].to_string();
    let mut after = excerpt[end..context_end].to_string();
    if context_end < excerpt.len() {
        after.push('…');
    }
    (before, matched, after)
}

fn find_casefolded_range(input: &str, needle: &str) -> Option<(usize, usize)> {
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return None;
    }
    let folded: Vec<(char, usize, usize)> = input
        .char_indices()
        .flat_map(|(start, character)| {
            let end = start + character.len_utf8();
            character
                .to_lowercase()
                .map(move |lowered| (lowered, start, end))
        })
        .collect();
    let index = folded.windows(needle.len()).position(|window| {
        window
            .iter()
            .map(|(character, _, _)| *character)
            .eq(needle.iter().copied())
    })?;
    Some((folded[index].1, folded[index + needle.len() - 1].2))
}

fn actions_for_result(result: Option<&SearchResult>) -> ModelRc<ActionRow> {
    ModelRc::new(VecModel::from(result.map_or_else(Vec::new, |result| {
        result.actions.iter().map(action_row).collect::<Vec<_>>()
    })))
}

fn passage_actions() -> ModelRc<ActionRow> {
    ModelRc::new(VecModel::from(vec![
        ActionRow {
            id: "passage.open-source".into(),
            title: "Show Passage + Open Source".into(),
            accessible_name: "Reopen current evidence, show its cited passage, and open its source when available".into(),
        },
        ActionRow {
            id: "passage.copy-citation".into(),
            title: "Copy Citation".into(),
            accessible_name: "Reopen the evidence and copy its citation".into(),
        },
        ActionRow {
            id: "passage.copy-excerpt".into(),
            title: "Copy Passage".into(),
            accessible_name: "Reopen the evidence and copy its passage".into(),
        },
    ]))
}

fn path_actions() -> ModelRc<ActionRow> {
    ModelRc::new(VecModel::from(vec![ActionRow {
        id: "path.copy".into(),
        title: "Copy Path".into(),
        accessible_name: "Copy the authorized matching file path".into(),
    }]))
}

pub(super) fn activate_passage_action(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    result_id: String,
    action_id: String,
) {
    if !matches!(
        action_id.as_str(),
        "passage.open-source" | "passage.copy-citation" | "passage.copy-excerpt"
    ) {
        show_notice(&ui, "This passage action is not available.".to_string());
        return;
    }
    if !passage_result_is_visible(&frontend, &result_id) {
        show_notice(
            &ui,
            "This passage is no longer visible in the current search.".to_string(),
        );
        return;
    }
    let generation = frontend.generation.load(Ordering::Acquire);
    let accepted = lock(&frontend.model)
        .accepted_passages
        .iter()
        .find(|accepted| accepted.result_id == result_id)
        .cloned();
    let Some(accepted) = accepted else {
        show_notice(&ui, "This passage belongs to an older search.".to_string());
        return;
    };
    if frontend.generation.load(Ordering::Acquire) != generation {
        show_notice(&ui, "This passage belongs to an older search.".to_string());
        return;
    }
    if let Err(error) = state.begin_passage_action(generation) {
        show_notice(&ui, error.message);
        return;
    }
    let config = state
        .settings()
        .ok()
        .and_then(|settings| settings.search_service());
    let Some(config) = config else {
        state.finish_action();
        show_notice(&ui, "Document search is not configured.".to_string());
        return;
    };

    runtime.spawn(async move {
        let prepared = super::passages::reopen(config, &accepted.passage)
            .await
            .map_err(action_error)
            .and_then(|reopened| match action_id.as_str() {
                "passage.copy-citation" => Ok(PassageAction::Copy(
                    reopened.citation,
                    "Copied citation.".to_string(),
                )),
                "passage.copy-excerpt" => Ok(PassageAction::Copy(
                    reopened.excerpt,
                    "Copied reopened passage.".to_string(),
                )),
                "passage.open-source" => Ok(PassageAction::Open {
                    path: reopened.path,
                    pdf_page: reopened.pdf_page,
                    excerpt: reopened.excerpt,
                    citation: reopened.citation,
                    fallback: reopened.fallback,
                }),
                _ => Err("This passage action is not available.".to_string()),
            });
        let failed_delivery_state = Arc::clone(&state);
        if slint::invoke_from_event_loop(move || {
            let still_accepted = lock(&frontend.model)
                .accepted_passages
                .iter()
                .any(|current| {
                    current.result_id == accepted.result_id
                        && current.passage.evidence_id == accepted.passage.evidence_id
                        && current.passage.artifact_version == accepted.passage.artifact_version
                });
            let current_generation = frontend.generation.load(Ordering::Acquire) == generation
                && state.ensure_generation(generation).is_ok()
                && still_accepted;
            if !current_generation {
                state.finish_action();
                show_notice(&ui, "This passage belongs to an older search.".to_string());
                return;
            }
            let outcome = match prepared {
                Ok(PassageAction::Copy(value, confirmation)) => {
                    super::platform::copy_text(&value).map(|()| confirmation)
                }
                Ok(PassageAction::Open {
                    path,
                    pdf_page,
                    excerpt,
                    citation,
                    fallback,
                }) => {
                    path.as_deref()
                        .map_or(Ok(()), super::platform::validate_selected_path)
                        .and_then(|()| {
                            if let Some(window) = ui.upgrade() {
                                show_reopened_passage(
                                    &window,
                                    &frontend,
                                    &accepted,
                                    &citation,
                                    &excerpt,
                                );
                            }
                            match path {
                                Some(path) => match pdf_page {
                                    Some(page) => crate::platform::open_local_pdf_page(&path, page),
                                    None => crate::platform::open_local_file(&path),
                                }
                                .map(|()| format!("Opened the source. {fallback}")),
                                None => Ok(
                                    "No local source file is available; the reopened passage is shown here."
                                        .to_string(),
                                ),
                            }
                        })
                }
                Err(message) => Err(LauncherError::file_unavailable(message)),
            };
            state.finish_action();
            match outcome {
                Ok(message) => {
                    if let Some(window) = ui.upgrade() {
                        window.set_notice(message.clone().into());
                        window.set_status_kind("ready".into());
                        window.set_status_message(message.into());
                    }
                }
                Err(error) => show_notice(&ui, error.message),
            }
        })
        .is_err()
        {
            failed_delivery_state.finish_action();
        }
    });
}

fn action_error(error: ReopenError) -> String {
    match error {
        ReopenError::Unavailable => {
            "The document could not be reopened; search service may be unavailable.".to_string()
        }
        ReopenError::Changed => {
            "The source changed or access was revoked. Search again before taking action."
                .to_string()
        }
    }
}

enum PassageAction {
    Copy(String, String),
    Open {
        path: Option<std::path::PathBuf>,
        pdf_page: Option<u32>,
        excerpt: String,
        citation: String,
        fallback: String,
    },
}

fn action_row(action: &Action) -> ActionRow {
    ActionRow {
        id: action.id.clone().into(),
        title: action.title.clone().into(),
        accessible_name: format!("{} action", action.title).into(),
    }
}

pub(super) fn file_actions() -> ModelRc<ActionRow> {
    ModelRc::new(VecModel::from(vec![
        ActionRow {
            id: "file.open".into(),
            title: "Open".into(),
            accessible_name: "Open selected file".into(),
        },
        ActionRow {
            id: "file.copy-path".into(),
            title: "Copy Path".into(),
            accessible_name: "Copy selected file path".into(),
        },
    ]))
}

pub(super) fn primary_action(result: &SearchResult) -> Option<&str> {
    result
        .actions
        .iter()
        .find(|action| action.primary)
        .map(|action| action.id.as_str())
}

fn result_kind_label(kind: &ResultKind) -> &'static str {
    match kind {
        ResultKind::Application => "application",
        ResultKind::Command => "command",
        ResultKind::Calculation => "calculation",
    }
}
