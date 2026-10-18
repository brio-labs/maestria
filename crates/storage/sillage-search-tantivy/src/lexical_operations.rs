use crate::tantivy_index::TantivyFullTextIndex;
use sillage_ports::{IndexedLexicalCard, IndexedLexicalChunk, PortError};

impl TantivyFullTextIndex {
    pub(crate) fn do_index_lexical_chunks(
        &self,
        chunks: Vec<IndexedLexicalChunk>,
    ) -> Result<(), PortError> {
        self.index_artifact_chunks_impl(Vec::new(), Vec::new(), chunks, Vec::new())
    }

    pub(crate) fn do_index_lexical_cards(
        &self,
        cards: Vec<IndexedLexicalCard>,
    ) -> Result<(), PortError> {
        self.index_artifact_chunks_impl(Vec::new(), Vec::new(), Vec::new(), cards)
    }
}
