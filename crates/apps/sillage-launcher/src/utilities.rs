//! Local quicklink/snippet storage and explicitly captured, memory-only clipboard history.
//!
//! Quicklinks accept absolute HTTP(S) URLs with at most one exact `{query}` placeholder,
//! percent-encoded as a URI component; snippets expand exact `{query}` and the exact UTC tokens
//! `{date:UTC:ISO8601}`, `{time:UTC:ISO8601}`, and `{datetime:UTC:ISO8601}`. All other snippet
//! text, including unknown or malformed braced text, remains literal. UTC values use Gregorian
//! dates in years 0001–9999 and whole-second times (`Z` suffix).
//! Save action, call `purge_expired` on a timer while the panel is hidden, use `visit_entries`
//! for borrowed list rows and `entry` for selected content, and dispatch returned effects. This
//! store never reads the clipboard, monitors keystrokes, detects secrets, or performs effects;
//! users must not save secrets.

mod clipboard;
mod entry;
mod persistence;
mod quicklinks;
mod snippets;
mod store;
mod utc_clock;

pub(crate) use entry::{UtilityEffect, UtilityKind};
pub(crate) use snippets::UtcSnapshot;
pub(crate) use store::Utilities;

pub(super) const MAX_EXPANSION_BYTES: usize = 64 * 1024;

#[cfg(test)]
#[path = "utilities/tests.rs"]
mod tests;
