use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use slint::{ModelRc, VecModel};

use super::super::passages::{Passage, PassageSearchResult};
use super::super::{
    AcceptedPassage, AcceptedPath, DisplayedResult, Frontend, LauncherWindow, empty_results, lock,
};
use super::{result_row, update_selected_actions};
use crate::ResultRow;

pub(super) fn apply_passages(
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
    model: &super::super::FrontendModel,
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
pub(in crate::application) fn apply_result_filter(
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
