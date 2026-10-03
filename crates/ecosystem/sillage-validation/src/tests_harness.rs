use super::test_fixtures::*;
use super::*;

#[test]
fn harness_run_validator_passes_for_successful_exit_code() {
    let fixture = ContextFixture {
        harness_exit_code: Some(0),
        ..ContextFixture::default()
    };

    let check = HarnessRunValidator.validate(&fixture.context());

    assert!(check.passed);
    assert_eq!(check.name, "harness_run");
}

#[test]
fn harness_run_validator_passes_when_no_exit_code_is_present() {
    let fixture = ContextFixture::default();

    let check = HarnessRunValidator.validate(&fixture.context());

    assert!(check.passed);
    assert!(check.message.contains("no harness run"));
}

#[test]
fn harness_run_validator_fails_for_non_zero_exit_code() {
    let fixture = ContextFixture {
        harness_exit_code: Some(2),
        ..ContextFixture::default()
    };

    let check = HarnessRunValidator.validate(&fixture.context());

    assert!(!check.passed);
    assert!(check.message.contains("2"));
}
