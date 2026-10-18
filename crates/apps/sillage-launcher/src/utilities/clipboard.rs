use std::time::Duration;
use tokio::time::Instant as MonotonicInstant;

use crate::errors::LauncherError;

use super::entry::{Entry, UtilityKind};

pub(super) const MAX_CLIPBOARD_ENTRIES: usize = 100;
pub(super) const MAX_CLIPBOARD_ITEM_BYTES: usize = 64 * 1024;
pub(super) const CLIPBOARD_RETENTION: Duration = Duration::from_secs(60 * 60);

struct CapturedEntry {
    entry: Entry,
    captured_at: MonotonicInstant,
}

pub(super) struct ClipboardHistory {
    entries: Vec<CapturedEntry>,
}

impl ClipboardHistory {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(super) fn capture(
        &mut self,
        text: &str,
        persistent_entries: &[Entry],
    ) -> Result<(), LauncherError> {
        self.capture_at(text, persistent_entries, MonotonicInstant::now())
    }

    pub(super) fn capture_at(
        &mut self,
        text: &str,
        persistent_entries: &[Entry],
        captured_at: MonotonicInstant,
    ) -> Result<(), LauncherError> {
        if text.is_empty() {
            return Err(LauncherError::invalid_request(
                "Clipboard capture cannot be empty",
            ));
        }
        if text.len() > MAX_CLIPBOARD_ITEM_BYTES {
            return Err(LauncherError::invalid_request(
                "Clipboard capture exceeds the 64 KiB item limit",
            ));
        }

        self.purge_expired_at(captured_at);
        if let Some(index) = self
            .entries
            .iter()
            .position(|record| record.entry.content == text)
        {
            let mut existing = self.entries.remove(index);
            existing.captured_at = captured_at;
            self.entries.push(existing);
            return Ok(());
        }

        let id = Entry::unique_id(|candidate| {
            persistent_entries.iter().any(|entry| entry.id == candidate)
                || self
                    .entries
                    .iter()
                    .any(|record| record.entry.id == candidate)
        });
        if self.entries.len() >= MAX_CLIPBOARD_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push(CapturedEntry {
            entry: Entry::new(
                id,
                UtilityKind::Clipboard,
                "Clipboard capture".to_string(),
                text.to_string(),
            ),
            captured_at,
        });
        Ok(())
    }

    pub(super) fn visit(&self, mut visit: impl FnMut(&Entry)) {
        for record in self.entries.iter().rev() {
            visit(&record.entry);
        }
    }

    pub(super) fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|record| record.entry.id == id)
            .map(|record| &record.entry)
    }

    pub(super) fn contains_id(&self, id: &str) -> bool {
        self.entries.iter().any(|record| record.entry.id == id)
    }

    pub(super) fn remove(&mut self, id: &str) -> bool {
        self.purge_expired();
        if let Some(index) = self.entries.iter().position(|record| record.entry.id == id) {
            self.entries.remove(index);
            true
        } else {
            false
        }
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn purge_expired(&mut self) -> bool {
        self.purge_expired_at(MonotonicInstant::now())
    }

    pub(super) fn purge_expired_at(&mut self, now: MonotonicInstant) -> bool {
        let previous_len = self.entries.len();
        self.entries.retain(|record| {
            now.saturating_duration_since(record.captured_at) < CLIPBOARD_RETENTION
        });
        self.entries.len() != previous_len
    }
}
