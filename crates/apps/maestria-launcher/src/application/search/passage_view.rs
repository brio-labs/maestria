use slint::{ModelRc, VecModel};

use super::super::{
    AcceptedPassage, DisplayedResult, Frontend, LauncherWindow, empty_results, lock,
};
use crate::ResultRow;
pub(in crate::application) fn open_passage_view(
    window: &LauncherWindow,
    frontend: &Frontend,
    result_id: &str,
) {
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

pub(super) fn show_reopened_passage(
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

pub(in crate::application) fn close_passage_view(window: &LauncherWindow, frontend: &Frontend) {
    lock(&frontend.model).content_view_passages.clear();
    window.set_passage_view_open(false);
    window.set_passage_view_results(empty_results());
    window.invoke_focus_search();
}

pub(in crate::application) fn passage_result_is_visible(
    frontend: &Frontend,
    result_id: &str,
) -> bool {
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

pub(in crate::application) fn path_result_is_visible(frontend: &Frontend, result_id: &str) -> bool {
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
