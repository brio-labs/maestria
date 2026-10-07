use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};

use crate::errors::LauncherError;

use super::clipboard::ClipboardHistory;
use super::entry::{self, Entry, UtilityEffect, UtilityKind};
use super::persistence::Persistence;
use super::{quicklinks, snippets};

pub(crate) struct Utilities {
    persistence: Persistence,
    entries: Vec<Entry>,
    clipboard: ClipboardHistory,
}

impl Utilities {
    pub(crate) fn load(
        directory: Result<std::path::PathBuf, String>,
    ) -> Result<Self, LauncherError> {
        let (persistence, entries) = Persistence::load(directory)?;
        Ok(Self {
            persistence,
            entries,
            clipboard: ClipboardHistory::new(),
        })
    }

    /// Visits the selected kind by reference and purges clipboard captures on every access.
    pub(crate) fn visit_entries(&mut self, kind: UtilityKind, mut visit: impl FnMut(&Entry)) {
        self.purge_expired();
        if kind == UtilityKind::Clipboard {
            self.clipboard.visit(visit);
        } else {
            for entry in self.entries.iter().filter(|entry| entry.kind == kind) {
                visit(entry);
            }
        }
    }

    /// Borrows one selected entry after removing expired clipboard content.
    pub(crate) fn entry(&mut self, kind: UtilityKind, id: &str) -> Result<&Entry, LauncherError> {
        self.purge_expired();
        let entry = if kind == UtilityKind::Clipboard {
            self.clipboard.entry(id)
        } else {
            self.entries
                .iter()
                .find(|entry| entry.id == id && entry.kind == kind)
        };
        entry.ok_or_else(|| LauncherError::stale_result("The utility entry no longer exists"))
    }

    pub(crate) fn purge_expired(&mut self) -> bool {
        self.clipboard.purge_expired()
    }

    pub(crate) fn upsert(
        &mut self,
        kind: UtilityKind,
        id: &str,
        title: &str,
        content: &str,
    ) -> Result<(), LauncherError> {
        if kind == UtilityKind::Clipboard {
            return Err(LauncherError::invalid_request(
                "Clipboard entries are memory-only and cannot be edited",
            ));
        }
        self.purge_expired();
        let id = if id.is_empty() {
            if self.entries.len() >= entry::MAX_ENTRIES {
                return Err(LauncherError::invalid_request(
                    "The utilities store has reached its entry limit",
                ));
            }
            Entry::unique_id(|candidate| {
                self.entries.iter().any(|stored| stored.id == candidate)
                    || self.clipboard.contains_id(candidate)
            })
        } else {
            id.to_string()
        };
        entry::validate_data(kind, &id, title, content)?;
        if self.clipboard.contains_id(&id) {
            return Err(LauncherError::invalid_request(
                "The utility ID is already in use",
            ));
        }

        let existing = self.entries.iter().position(|stored| stored.id == id);
        if let Some(index) = existing {
            if self.entries[index].kind != kind {
                return Err(LauncherError::invalid_request(
                    "A utility ID cannot be reused for a different kind",
                ));
            }
        } else if self.entries.len() >= entry::MAX_ENTRIES {
            return Err(LauncherError::invalid_request(
                "The utilities store has reached its entry limit",
            ));
        }

        let new_entry = Entry::new(id, kind, title.to_string(), content.to_string());
        if existing.is_none() {
            self.entries.try_reserve(1).map_err(|error| {
                LauncherError::file_unavailable(format!(
                    "Memory for a new utility entry could not be reserved: {error}"
                ))
            })?;
        }
        let change = match existing {
            Some(index) => CandidateChange::Replace {
                index,
                entry: &new_entry,
            },
            None => CandidateChange::Append(&new_entry),
        };
        self.persistence.write(&CandidateEntries {
            entries: &self.entries,
            change,
        })?;

        match existing {
            Some(index) => self.entries[index] = new_entry,
            None => self.entries.push(new_entry),
        }
        Ok(())
    }

    pub(crate) fn remove(&mut self, kind: UtilityKind, id: &str) -> Result<(), LauncherError> {
        entry::validate_id(id)?;
        self.purge_expired();
        if kind == UtilityKind::Clipboard {
            return if self.clipboard.remove(id) {
                Ok(())
            } else {
                Err(LauncherError::invalid_request(
                    "The clipboard entry no longer exists",
                ))
            };
        }

        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.id == id && entry.kind == kind)
        else {
            return Err(LauncherError::invalid_request(
                "The utility entry does not exist",
            ));
        };
        self.persistence.write(&CandidateEntries {
            entries: &self.entries,
            change: CandidateChange::Remove { index },
        })?;
        self.entries.remove(index);
        Ok(())
    }

    pub(crate) fn clear_clipboard(&mut self) {
        self.clipboard.clear();
    }

    pub(crate) fn capture_clipboard(&mut self, text: &str) -> Result<(), LauncherError> {
        self.clipboard.capture(text, &self.entries)
    }

    pub(crate) fn expand(
        &mut self,
        id: &str,
        argument: &str,
    ) -> Result<UtilityEffect, LauncherError> {
        self.purge_expired();
        if let Some(entry) = self.clipboard.entry(id) {
            return Ok(UtilityEffect::Copy(entry.content.clone()));
        }

        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| LauncherError::stale_result("The utility entry no longer exists"))?;
        match entry.kind {
            UtilityKind::Quicklink => Ok(UtilityEffect::OpenUri(quicklinks::expand(
                &entry.content,
                argument,
            )?)),
            UtilityKind::Snippet => Ok(UtilityEffect::Copy(snippets::expand(
                &entry.content,
                argument,
            )?)),
            UtilityKind::Clipboard => Err(LauncherError::stale_result(
                "The clipboard entry no longer exists",
            )),
        }
    }
}

struct CandidateEntries<'a> {
    entries: &'a [Entry],
    change: CandidateChange<'a>,
}

#[derive(Clone, Copy)]
enum CandidateChange<'a> {
    Append(&'a Entry),
    Replace { index: usize, entry: &'a Entry },
    Remove { index: usize },
}

impl Serialize for CandidateEntries<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let length = match self.change {
            CandidateChange::Append(_) => self.entries.len() + 1,
            CandidateChange::Replace { .. } => self.entries.len(),
            CandidateChange::Remove { .. } => self.entries.len() - 1,
        };
        let mut sequence = serializer.serialize_seq(Some(length))?;
        for (index, existing) in self.entries.iter().enumerate() {
            match self.change {
                CandidateChange::Replace {
                    index: replacement_index,
                    entry,
                } if index == replacement_index => sequence.serialize_element(entry)?,
                CandidateChange::Remove {
                    index: removal_index,
                } if index == removal_index => continue,
                _ => sequence.serialize_element(existing)?,
            }
        }
        if let CandidateChange::Append(entry) = self.change {
            sequence.serialize_element(entry)?;
        }
        sequence.end()
    }
}
