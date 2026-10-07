use super::super::{DisplayedResult, Frontend, LauncherWindow, lock};
use super::update_selected_actions;

pub(in crate::application) fn navigate_result_selection(
    window: &LauncherWindow,
    frontend: &Frontend,
    forward: bool,
) {
    let next = {
        let model = lock(&frontend.model);
        next_result_index(
            &model.displayed,
            window.get_selected_index().max(0) as usize,
            forward,
        )
    };
    if let Some(next) = next {
        window.set_selected_index(next as i32);
        if !window.get_actions_open() {
            update_selected_actions(window, frontend, next);
        }
    }
}

pub(super) fn retained_result_index(
    rows: &[crate::ResultRow],
    previous_id: Option<&str>,
    fallback: usize,
) -> usize {
    if let Some(previous_id) = previous_id
        && let Some(index) = rows
            .iter()
            .position(|row| row.id.as_str() == previous_id && row.kind.as_str() != "passage_group")
    {
        return index;
    }
    if rows
        .get(fallback)
        .is_some_and(|row| row.kind.as_str() != "passage_group")
    {
        return fallback;
    }
    for (index, row) in rows.iter().enumerate() {
        if row.kind.as_str() != "passage_group" {
            return index;
        }
    }
    0
}

fn next_result_index(
    displayed: &[DisplayedResult],
    current: usize,
    forward: bool,
) -> Option<usize> {
    if forward {
        displayed
            .iter()
            .enumerate()
            .skip(current + 1)
            .find(|(_, result)| !matches!(result, DisplayedResult::Group))
            .map(|(index, _)| index)
    } else {
        displayed
            .iter()
            .enumerate()
            .take(current)
            .rev()
            .find(|(_, result)| !matches!(result, DisplayedResult::Group))
            .map(|(index, _)| index)
    }
}

#[cfg(test)]
mod tests {
    use super::{DisplayedResult, next_result_index, retained_result_index};

    #[test]
    fn arrows_skip_document_headers_in_both_directions() {
        let rows = [
            DisplayedResult::Application(0),
            DisplayedResult::Group,
            DisplayedResult::Passage(0),
            DisplayedResult::Group,
            DisplayedResult::Passage(1),
            DisplayedResult::Path(0),
        ];
        assert_eq!(next_result_index(&rows, 0, true), Some(2));
        assert_eq!(next_result_index(&rows, 2, true), Some(4));
        assert_eq!(next_result_index(&rows, 4, true), Some(5));
        assert_eq!(next_result_index(&rows, 5, false), Some(4));
        assert_eq!(next_result_index(&rows, 4, false), Some(2));
        assert_eq!(next_result_index(&rows, 2, false), Some(0));
    }

    #[test]
    fn arrows_stay_at_actionable_edges_and_ignore_header_only_results() {
        let rows = [
            DisplayedResult::Group,
            DisplayedResult::Passage(0),
            DisplayedResult::Group,
        ];
        assert_eq!(next_result_index(&rows, 1, false), None);
        assert_eq!(next_result_index(&rows, 1, true), None);
        assert_eq!(next_result_index(&[], 0, true), None);
        assert_eq!(next_result_index(&[], 0, false), None);
        assert_eq!(next_result_index(&[DisplayedResult::Group], 0, true), None);
        assert_eq!(next_result_index(&[DisplayedResult::Group], 0, false), None);
    }

    fn row(id: &str, kind: &str) -> crate::ResultRow {
        crate::ResultRow {
            id: id.into(),
            kind: kind.into(),
            title: "".into(),
            subtitle: "".into(),
            accessible_name: "".into(),
            excerpt_before: "".into(),
            excerpt_match: "".into(),
            excerpt_after: "".into(),
            content: "".into(),
        }
    }

    #[test]
    fn refresh_retains_identity_when_rows_move_or_preceding_results_disappear() {
        let rows = [
            row("group", "passage_group"),
            row("selected", "passage"),
            row("other", "application"),
        ];
        assert_eq!(retained_result_index(&rows, Some("selected"), 2), 1);
        assert_eq!(retained_result_index(&rows, Some("other"), 0), 2);
    }

    #[test]
    fn removed_selection_falls_back_to_actionable_row_not_header() {
        let rows = [row("group", "passage_group"), row("remaining", "path")];
        assert_eq!(retained_result_index(&rows, Some("removed"), 0), 1);
        assert_eq!(retained_result_index(&rows, Some("removed"), 1), 1);
        assert_eq!(retained_result_index(&[], Some("removed"), 0), 0);
    }
}
