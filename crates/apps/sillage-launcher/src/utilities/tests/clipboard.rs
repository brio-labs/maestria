use std::error::Error;
use std::time::Duration;
use tokio::time::Instant as MonotonicInstant;

use super::super::clipboard::{
    CLIPBOARD_RETENTION, ClipboardHistory, MAX_CLIPBOARD_ENTRIES, MAX_CLIPBOARD_ITEM_BYTES,
};

#[test]
fn capture_is_bounded_deduplicated_lru_and_removable() -> Result<(), Box<dyn Error>> {
    let mut clipboard = ClipboardHistory::new();
    clipboard.capture(&"x".repeat(MAX_CLIPBOARD_ITEM_BYTES), &[])?;
    assert!(
        clipboard
            .capture(&"x".repeat(MAX_CLIPBOARD_ITEM_BYTES + 1), &[])
            .is_err()
    );
    assert!(clipboard.capture("", &[]).is_err());
    clipboard.clear();
    clipboard.capture("duplicate", &[])?;
    let first_id = {
        let mut ids = Vec::new();
        clipboard.visit(|entry| ids.push(entry.id.clone()));
        ids.remove(0)
    };
    clipboard.capture("duplicate", &[])?;
    let repeated_id = {
        let mut ids = Vec::new();
        clipboard.visit(|entry| ids.push(entry.id.clone()));
        ids.remove(0)
    };
    assert_eq!(first_id, repeated_id);

    for index in 0..=MAX_CLIPBOARD_ENTRIES {
        clipboard.capture(&format!("item-{index}"), &[])?;
    }
    let recent = {
        let mut values = Vec::new();
        clipboard.visit(|entry| values.push(entry.content.clone()));
        values
    };
    assert_eq!(recent.len(), MAX_CLIPBOARD_ENTRIES);
    assert!(!recent.iter().any(|content| content == "item-0"));

    let first_recent_id = {
        let mut ids = Vec::new();
        clipboard.visit(|entry| ids.push(entry.id.clone()));
        ids.remove(0)
    };
    assert!(clipboard.remove(&first_recent_id));
    assert_eq!(
        {
            let mut count = 0;
            clipboard.visit(|_| count += 1);
            count
        },
        MAX_CLIPBOARD_ENTRIES - 1
    );
    Ok(())
}

#[test]
fn history_uses_monotonic_one_hour_expiry_and_physically_removes_content()
-> Result<(), Box<dyn Error>> {
    let mut clipboard = ClipboardHistory::new();
    let captured_at = MonotonicInstant::now();
    clipboard.capture_at("expire this", &[], captured_at)?;
    let id = {
        let mut ids = Vec::new();
        clipboard.visit(|entry| ids.push(entry.id.clone()));
        ids.remove(0)
    };

    assert!(
        !clipboard.purge_expired_at(captured_at + CLIPBOARD_RETENTION - Duration::from_nanos(1))
    );
    assert!(clipboard.entry(&id).is_some());
    assert!(clipboard.purge_expired_at(captured_at + CLIPBOARD_RETENTION));
    assert!(clipboard.entry(&id).is_none());
    let mut entries = 0;
    clipboard.visit(|_| entries += 1);
    assert_eq!(entries, 0);
    Ok(())
}
