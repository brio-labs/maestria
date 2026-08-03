//! In-memory lexical search lanes (Rule 13: one concept per module).
//!
//! This façade owns the shared lane contract and re-exports the two corpus
//! lanes — chunks and cards — together with their index maintenance. Field
//! matching (query preparation, per-field scoring, hit metadata) lives in
//! [`matching`], the generic lane pipeline in [`lane`], and each corpus lane
//! (identity, field extraction, metering, hit construction) in [`chunks`]
//! and [`cards`].

#[path = "lexical_cards.rs"]
mod cards;
#[path = "lexical_chunks.rs"]
mod chunks;
#[path = "lexical_lane.rs"]
mod lane;
#[path = "lexical_matching.rs"]
mod matching;

pub(crate) use cards::{index_lexical_cards, search_cards_lexical, search_cards_lexical_filtered};
pub(crate) use chunks::{index_lexical_chunks, search_lexical, search_lexical_filtered};

#[cfg(test)]
#[path = "lexical_tests.rs"]
mod tests;
