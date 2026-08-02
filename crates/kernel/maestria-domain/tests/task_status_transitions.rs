use maestria_domain::TaskStatus;

#[test]
fn draft_walks_open_active_validating() {
    assert_eq!(
        TaskStatus::Draft.path_to_validating(),
        Some(vec![
            TaskStatus::Open,
            TaskStatus::Active,
            TaskStatus::Validating,
        ])
    );
}

#[test]
fn open_and_blocked_walk_active_validating() {
    for status in [TaskStatus::Open, TaskStatus::Blocked] {
        assert_eq!(
            status.path_to_validating(),
            Some(vec![TaskStatus::Active, TaskStatus::Validating])
        );
    }
}

#[test]
fn active_walks_validating_directly() {
    assert_eq!(
        TaskStatus::Active.path_to_validating(),
        Some(vec![TaskStatus::Validating])
    );
}

#[test]
fn validating_is_already_on_the_path() {
    assert_eq!(TaskStatus::Validating.path_to_validating(), Some(vec![]));
}

#[test]
fn terminal_statuses_have_no_validation_path() {
    for status in [
        TaskStatus::CompletedVerified,
        TaskStatus::CompletedWithWarnings,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ] {
        assert_eq!(status.path_to_validating(), None, "status {status:?}");
    }
}
