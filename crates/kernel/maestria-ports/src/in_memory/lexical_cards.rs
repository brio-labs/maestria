//! The card corpus lane: identity, field extraction, metering, and hit
//! construction for [`IndexedLexicalCard`], plus the card-facing entry
//! points. Search and index behavior is the generic pipeline in
//! [`super::lane`] (Rule 16: cross-lane behavior crosses typed functions).

use super::super::execution::saturating_u64;
use super::lane::{LexicalLane, index_lane, search_lane};
use crate::lexical::{
    CardField, IndexedLexicalCard, LexicalCardHit, LexicalHitMetadata, LexicalQuery,
};
use crate::{BoundedSearch, PortError};
use maestria_domain::{ArtifactId, CardId};
use std::sync::{Arc, Mutex};

pub(crate) struct CardLane;

impl LexicalLane for CardLane {
    type Id = CardId;
    type Field = CardField;
    type Record = IndexedLexicalCard;
    type Hit = LexicalCardHit;

    fn id(record: &Self::Record) -> Self::Id {
        record.card_id
    }

    fn artifact_id(record: &Self::Record) -> ArtifactId {
        record.artifact_id
    }

    fn is_id_field(field: &Self::Field) -> bool {
        matches!(field, CardField::Id)
    }

    fn id_key(record: &Self::Record) -> String {
        format!(
            "card:{}:{}",
            record.artifact_id.value(),
            record.card_id.value()
        )
    }

    fn field_value<'a>(record: &'a Self::Record, field: &Self::Field) -> Option<&'a String> {
        match field {
            CardField::Title => Some(&record.title),
            CardField::Body => Some(&record.body),
            CardField::Path => record.path.as_ref(),
            CardField::Filename => record.filename.as_ref(),
            CardField::Symbol => record.symbol.as_ref(),
            CardField::Id => None,
        }
    }

    fn field_len(record: &Self::Record, field: &Self::Field) -> usize {
        match field {
            CardField::Title => record.title.len(),
            CardField::Body => record.body.len(),
            CardField::Path => record.path.as_ref().map_or(0, String::len),
            CardField::Filename => record.filename.as_ref().map_or(0, String::len),
            CardField::Symbol => record.symbol.as_ref().map_or(0, String::len),
            CardField::Id => 0,
        }
    }

    fn metered_bytes(record: &Self::Record) -> u64 {
        saturating_u64(record.title.len().saturating_add(record.body.len()))
            .saturating_add(
                record
                    .path
                    .as_ref()
                    .map_or(0, |value| saturating_u64(value.len())),
            )
            .saturating_add(
                record
                    .filename
                    .as_ref()
                    .map_or(0, |value| saturating_u64(value.len())),
            )
            .saturating_add(
                record
                    .symbol
                    .as_ref()
                    .map_or(0, |value| saturating_u64(value.len())),
            )
    }

    fn build_hit(record: Self::Record, metadata: LexicalHitMetadata) -> Self::Hit {
        LexicalCardHit {
            card: record,
            metadata,
        }
    }

    fn hit_score(hit: &Self::Hit) -> f32 {
        hit.metadata.raw_score
    }

    fn hit_artifact_id(hit: &Self::Hit) -> ArtifactId {
        hit.card.artifact_id
    }

    fn hit_item_id(hit: &Self::Hit) -> Self::Id {
        hit.card.card_id
    }

    fn set_hit_rank(hit: &mut Self::Hit, rank: u32) {
        hit.metadata.raw_rank = rank;
    }
}

pub(crate) fn index_lexical_cards(
    lexical_cards: &Arc<Mutex<Vec<IndexedLexicalCard>>>,
    cards: Vec<IndexedLexicalCard>,
) -> Result<(), PortError> {
    index_lane::<CardLane>(lexical_cards, cards)
}

pub(crate) fn search_cards_lexical(
    lexical_cards: &Arc<Mutex<Vec<IndexedLexicalCard>>>,
    query: LexicalQuery<CardField>,
) -> Result<BoundedSearch<LexicalCardHit>, PortError> {
    search_cards_lexical_filtered(lexical_cards, query, &|_, _| Ok(true))
}

pub(crate) fn search_cards_lexical_filtered(
    lexical_cards: &Arc<Mutex<Vec<IndexedLexicalCard>>>,
    query: LexicalQuery<CardField>,
    filter: &dyn Fn(CardId, ArtifactId) -> Result<bool, PortError>,
) -> Result<BoundedSearch<LexicalCardHit>, PortError> {
    search_lane::<CardLane>(
        lexical_cards,
        query,
        filter,
        "lexical card search query must not be empty",
        "lexical card query has no fields",
        "lexical card result limit",
    )
}
