use super::test_helpers::adapter;
use maestria_ports::{HarnessAdapter, HarnessCommandClass};

#[tokio::test]
async fn capabilities_report_shell_only() -> Result<(), Box<dyn std::error::Error>> {
    let caps = adapter().capabilities()?;
    #[cfg(target_os = "linux")]
    assert!(caps.read_enabled);
    #[cfg(not(target_os = "linux"))]
    assert!(!caps.read_enabled);
    assert!(!caps.write_enabled);
    assert!(!caps.web_enabled);
    assert_eq!(caps.command_classes, vec![HarnessCommandClass::Shell]);
    Ok(())
}
