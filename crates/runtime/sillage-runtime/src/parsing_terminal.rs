use crate::config::EffectExecutionContext;
use crate::parser_mapping::domain_parse_status;
use sillage_domain::{ArtifactId, DomainInput, ParserResult};

impl EffectExecutionContext {
    /// Ask the domain to terminalize a non-indexable artifact.
    ///
    /// Unsupported and invalid-input results carry no chunks, so the
    /// full-text indexing step never emits `FullTextIndexCompleted` and
    /// the artifact would otherwise stay `Pending` forever. The domain's
    /// `StartFullTextIndex` recovery branch terminalizes zero-chunk
    /// artifacts with complete evidence coverage.
    pub(crate) async fn emit_start_full_text_index(
        &self,
        artifact_id: ArtifactId,
    ) -> Result<(), crate::FeedbackError> {
        Self::send_input_blocking(
            &self.input_tx,
            DomainInput::StartFullTextIndex(sillage_domain::StartFullTextIndex { artifact_id }),
            "start full-text index",
        )
        .await
    }

    pub(crate) async fn emit_terminal_parser_completed(
        &self,
        artifact_id: ArtifactId,
        artifact_version_id: sillage_domain::ArtifactVersionId,
        status: sillage_ports::ParseStatus,
        source_hash: &sillage_domain::ContentHash,
    ) -> bool {
        Self::send_input_blocking(
            &self.input_tx,
            DomainInput::ParserCompleted(ParserResult {
                artifact_id,
                artifact_version_id,
                content_hash: source_hash.clone(),
                status: domain_parse_status(status),
                tree_root_id: None,
                tree_nodes: Vec::new(),
                chunks: Vec::new(),
                cards: Vec::new(),
            }),
            "parser completion",
        )
        .await
        .is_ok()
    }
}
