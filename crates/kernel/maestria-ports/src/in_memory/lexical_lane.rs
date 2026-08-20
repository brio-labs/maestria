//! The generic in-memory lexical index lane (Rule 13/16: one shared
//! pipeline, typed per corpus). [`LexicalLane`] names everything a corpus
//! must provide — record identity, field extraction, and metering — so
//! index maintenance is implemented once and reused by the chunk and card
//! lanes instead of being duplicated per corpus type. The typed lexical
//! *search* family was removed with ADR-0005 (expiry v0.7.0).

use crate::PortError;
use crate::in_memory::store::lock_map;
use maestria_domain::ArtifactId;
use std::sync::{Arc, Mutex};

/// One indexable corpus lane: identity for a single indexed record type.
pub(super) trait LexicalLane {
    type Id: Ord + Clone + Copy + std::fmt::Debug;
    type Record: Clone;

    fn id(record: &Self::Record) -> Self::Id;
    fn artifact_id(record: &Self::Record) -> ArtifactId;
}

/// Replace-or-append indexing: an incoming record with the same (artifact,
/// item) identity as an existing one replaces it, preserving lane order.
pub(super) fn index_lane<R: LexicalLane>(
    store: &Arc<Mutex<Vec<R::Record>>>,
    records: Vec<R::Record>,
) -> Result<(), PortError> {
    let mut guard = lock_map(store, "lexical index lock poisoned")?;
    for record in &records {
        guard.retain(|existing| {
            R::artifact_id(existing) != R::artifact_id(record) || R::id(existing) != R::id(record)
        });
    }
    guard.extend(records);
    Ok(())
}
