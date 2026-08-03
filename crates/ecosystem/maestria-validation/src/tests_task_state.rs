use super::test_fixtures::*;
use super::*;
use maestria_domain::TaskStatus;

#[test]
fn task_state_validator_passes_for_validating_task() {
    let fixture = ContextFixture {
        task: Some(task(1, TaskStatus::Validating)),
        ..ContextFixture::default()
    };

    let check = TaskStateValidator.validate(&fixture.context());

    assert!(check.passed);
    assert_eq!(check.name, "task_state");
}

#[test]
fn task_state_validator_fails_for_non_validating_task() {
    let fixture = ContextFixture {
        task: Some(task(1, TaskStatus::Active)),
        ..ContextFixture::default()
    };

    let check = TaskStateValidator.validate(&fixture.context());

    assert!(!check.passed);
    assert!(check.message.contains("Validating"));
}

#[test]
fn task_state_validator_fails_without_task() {
    let fixture = ContextFixture::default();

    let check = TaskStateValidator.validate(&fixture.context());

    assert!(!check.passed);
    assert!(check.message.contains("task is required"));
}
