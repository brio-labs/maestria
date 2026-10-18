//! In-memory lexical index lanes (Rule 13: one concept per module).
//!
//! This façade owns the shared lane contract and re-exports the two corpus
//! lanes — chunks and cards — together with their index maintenance. The
//! typed lexical *search* family was removed with ADR-0005 (expiry v0.7.0).

#[path = "lexical_cards.rs"]
mod cards;
#[path = "lexical_chunks.rs"]
mod chunks;
#[path = "lexical_lane.rs"]
mod lane;

pub(crate) use cards::index_lexical_cards;
pub(crate) use chunks::index_lexical_chunks;
