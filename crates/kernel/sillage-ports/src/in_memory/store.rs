//! Map-backed in-memory store plumbing (Rule 13: one concept per module).
//!
//! Every in-memory store guards its maps with a `Mutex`; the lock-poison
//! error and guard acquisition are shared so the ~25 identical chains cannot
//! drift. Keys stay concrete domain ID types (Rule 27: no `Id` trait).

use crate::PortError;
use std::sync::{Mutex, MutexGuard};

/// The typed lock-poison error for an in-memory store map. `context` is the
/// full static context (e.g. `"chunk repository lock poisoned"`).
pub(super) fn poison(context: &'static str) -> PortError {
    PortError::InternalContext {
        context,
        source: "store mutex is poisoned".to_string(),
    }
}

/// Acquire the store's mutex, mapping poison to a typed error.
pub(super) fn lock_map<'a, T>(
    map: &'a Mutex<T>,
    context: &'static str,
) -> Result<MutexGuard<'a, T>, PortError> {
    map.lock().map_err(|_| poison(context))
}
