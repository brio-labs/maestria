use super::test_helpers::adapter;
use super::*;

#[tokio::test]
async fn capabilities_report_shell_only() -> Result<(), Box<dyn std::error::Error>> {
    let caps = adapter().capabilities()?;
    assert!(caps.read_enabled);
    assert!(!caps.write_enabled);
    assert!(!caps.web_enabled);
    assert_eq!(caps.command_classes, vec![HarnessCommandClass::Shell]);
    Ok(())
}
