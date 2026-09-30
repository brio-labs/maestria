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
    use super::{DisplayedResult, next_result_index};

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
}
