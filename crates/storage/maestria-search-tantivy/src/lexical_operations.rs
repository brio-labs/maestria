use crate::{
    error::to_port_error,
    keys::{card_key, chunk_key},
    tantivy_index::TantivyFullTextIndex,
};
use maestria_ports::{IndexedLexicalCard, IndexedLexicalChunk, PortError};
use tantivy::Term;

impl TantivyFullTextIndex {
    pub(crate) fn do_index_lexical_chunks(
        &self,
        chunks: Vec<IndexedLexicalChunk>,
    ) -> Result<(), PortError> {
        self.with_writer(
            "index lexical chunks requires a writable full-text index",
            |writer| {
                for chunk in chunks {
                    writer.delete_term(Term::from_field_text(
                        self.fields.key,
                        &chunk_key(chunk.artifact_id, chunk.chunk_id),
                    ));
                    writer
                        .add_document(self.lexical_chunk_document(chunk))
                        .map_err(to_port_error)?;
                }
                Ok(())
            },
        )
    }

    pub(crate) fn do_index_lexical_cards(
        &self,
        cards: Vec<IndexedLexicalCard>,
    ) -> Result<(), PortError> {
        self.with_writer(
            "index lexical cards requires a writable full-text index",
            |writer| {
                for card in cards {
                    writer.delete_term(Term::from_field_text(
                        self.fields.card_key,
                        &card_key(card.artifact_id, card.card_id),
                    ));
                    writer
                        .add_document(self.lexical_card_document(card))
                        .map_err(to_port_error)?;
                }
                Ok(())
            },
        )
    }
}
