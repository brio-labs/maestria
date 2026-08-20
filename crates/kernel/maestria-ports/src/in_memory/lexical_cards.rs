//! The card corpus lane: identity, field extraction, metering, and hit
//! construction for [`IndexedLexicalCard`], plus the card-facing entry
//! points. Search and index behavior is the generic pipeline in
//! [`super::lane`] (Rule 16: cross-lane behavior crosses typed functions).

use super::lane::{LexicalLane, index_lane};
use crate::PortError;
use crate::lexical::IndexedLexicalCard;
use maestria_domain::{ArtifactId, CardId};
use std::sync::{Arc, Mutex};

pub(crate) struct CardLane;

impl LexicalLane for CardLane {
    type Id = CardId;
    type Record = IndexedLexicalCard;

    fn id(record: &Self::Record) -> Self::Id {
        record.card_id
    }

    fn artifact_id(record: &Self::Record) -> ArtifactId {
        record.artifact_id
    }
}

pub(crate) fn index_lexical_cards(
    lexical_cards: &Arc<Mutex<Vec<IndexedLexicalCard>>>,
    cards: Vec<IndexedLexicalCard>,
) -> Result<(), PortError> {
    index_lane::<CardLane>(lexical_cards, cards)
}
