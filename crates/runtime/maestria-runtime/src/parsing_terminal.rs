use crate::config::EffectExecutionContext;
use crate::parser_mapping::domain_parse_status;
use maestria_domain::{ArtifactId, DomainInput, ParserResult};

impl EffectExecutionContext {
    pub(crate) fn emit_terminal_parser_completed(
        &self,
        artifact_id: ArtifactId,
        artifact_version_id: maestria_domain::ArtifactVersionId,
        status: maestria_ports::ParseStatus,
        source_hash: &maestria_domain::ContentHash,
    ) -> bool {
        Self::send_input(
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
        .is_ok()
    }
}
