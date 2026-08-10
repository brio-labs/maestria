use crate::config::EffectExecutionContext;
use crate::parser_mapping::domain_parse_status;
use crate::parsing_records::build_indexable_records;
use maestria_domain::{
    ArtifactId, BlobId, DomainInput, OcrDisclosure, OcrIntent, OcrProviderIdentity,
    OcrRetentionPolicy, ParseArtifactRequest, ParserResult,
};
use maestria_ports::{FileHandle, ParseContext, ParseOutcome};

impl EffectExecutionContext {
    /// Run the parser and emit the resulting domain inputs:
    /// `ParserCompleted`, `RecordEvidence` per chunk, and `StartFullTextIndex`.
    pub(super) async fn parse_and_emit(
        &self,
        request: &ParseArtifactRequest,
        artifact_id: ArtifactId,
        parse_bytes: Vec<u8>,
        blob_id: BlobId,
        source_hash: maestria_domain::ContentHash,
        path: std::path::PathBuf,
    ) -> bool {
        let file = FileHandle {
            path,
            bytes: parse_bytes,
        };
        let prior_ocr = {
            let state = self.state.read().await;
            // Iterate durable completions, not intents: a pending or failed intent
            // must never mask a matching completion. BTreeMap ordering keeps the
            // choice stable if multiple provider identities completed.
            state
                .ocr_results
                .iter()
                .filter_map(|(request_id, completion)| {
                    state
                        .ocr_intents
                        .get(request_id)
                        .filter(|intent| {
                            intent.artifact_id() == artifact_id
                                && intent.source_blob() == blob_id
                                && intent.source_hash().as_str() == source_hash.as_str()
                        })
                        .map(|_| completion)
                })
                .next()
                .cloned()
        };
        let parse_result = if let Some(completion) = prior_ocr {
            let pages = completion.pages().to_vec();
            self.adapters
                .parser
                .parse_with_ocr(file, ParseContext { artifact_id }, &pages)
                .map(ParseOutcome::Complete)
        } else {
            self.adapters
                .parser
                .parse_outcome(file, ParseContext { artifact_id })
        };
        match parse_result {
            Ok(ParseOutcome::Complete(parsed)) => {
                self.emit_parsed_artifact(request, artifact_id, parsed, blob_id, &source_hash)
                    .await
            }
            Ok(ParseOutcome::NeedsOcr { partial, pages }) => {
                if !self.request_ocr_if_configured(
                    artifact_id,
                    blob_id,
                    &source_hash,
                    pages.as_slice(),
                ) {
                    return false;
                }
                let pending = maestria_ports::ParsedArtifact {
                    status: maestria_ports::ParseStatus::NeedsOcr,
                    chunks: Vec::new(),
                    cards: Vec::new(),
                    ..partial
                };
                self.emit_parsed_artifact(request, artifact_id, pending, blob_id, &source_hash)
                    .await
            }
            Err(error) if error.is_invalid_input() => {
                tracing::warn!(
                    artifact_id = %artifact_id.value(),
                    "parser rejected artifact as invalid input"
                );
                if !self
                    .emit_terminal_parser_completed(
                        artifact_id,
                        maestria_domain::ArtifactVersionId::new(artifact_id.value()),
                        maestria_ports::ParseStatus::Failed,
                        &source_hash,
                    )
                    .await
                {
                    return false;
                }
                self.emit_start_full_text_index(artifact_id).await.is_ok()
            }
            Err(error) => {
                tracing::error!(artifact_id = %artifact_id, %error, "parser failed");
                if !self
                    .emit_terminal_parser_completed(
                        artifact_id,
                        maestria_domain::ArtifactVersionId::new(artifact_id.value()),
                        maestria_ports::ParseStatus::Failed,
                        &source_hash,
                    )
                    .await
                {
                    return false;
                }
                self.emit_start_full_text_index(artifact_id).await.is_ok()
            }
        }
    }

    fn request_ocr_if_configured(
        &self,
        artifact_id: ArtifactId,
        blob_id: BlobId,
        source_hash: &maestria_domain::ContentHash,
        pages: &[u32],
    ) -> bool {
        let Some(provider) = &self.adapters.ocr_provider else {
            return true;
        };
        let identity = provider.identity();
        let disclosure = provider.disclosure();
        let Ok(provider_identity) = OcrProviderIdentity::new(
            identity.provider,
            identity.model,
            identity.revision,
            identity.artifact_hash,
            identity.preprocessing_version,
        ) else {
            return false;
        };
        let retention = if matches!(
            disclosure.retention,
            maestria_ports::RetentionPolicy::NoRetention
        ) {
            OcrRetentionPolicy::NoRetention
        } else {
            OcrRetentionPolicy::ProviderDefined
        };
        let Ok(intent) = OcrIntent::new(
            artifact_id,
            blob_id,
            source_hash.clone(),
            pages.iter().copied(),
            provider_identity,
            OcrDisclosure::new(disclosure.remote, retention),
        ) else {
            return false;
        };
        Self::send_input(
            &self.input_tx,
            DomainInput::OcrRequested(maestria_domain::OcrRequested { intent }),
            "OCR request",
        )
        .is_ok()
    }

    async fn emit_parsed_artifact(
        &self,
        request: &ParseArtifactRequest,
        artifact_id: ArtifactId,
        parsed: maestria_ports::ParsedArtifact,
        blob_id: BlobId,
        source_hash: &maestria_domain::ContentHash,
    ) -> bool {
        if parsed.artifact_id != artifact_id {
            tracing::error!(
                requested_artifact_id = %artifact_id.value(),
                parsed_artifact_id = %parsed.artifact_id.value(),
                "parser returned a result for a different artifact; rejecting"
            );
            return false;
        }
        if parsed.content_hash.as_str() != source_hash.as_str() {
            tracing::error!(
                artifact_id = %artifact_id.value(),
                expected = %source_hash.as_str(),
                actual = %parsed.content_hash.as_str(),
                "parsed artifact content hash does not match source hash; rejecting"
            );
            return false;
        }
        let parser_status = parsed.status.clone();
        let indexable = matches!(
            parser_status,
            maestria_ports::ParseStatus::Parsed | maestria_ports::ParseStatus::MetadataOnly
        ) && (!parsed.chunks.is_empty() || !parsed.cards.is_empty());
        let status = domain_parse_status(parser_status.clone());
        if !indexable {
            tracing::warn!(
                artifact_id = %artifact_id.value(),
                status = ?parser_status,
                "parser returned a non-indexable status"
            );
        }
        let (evidence_inputs, chunks, cards) = if indexable {
            match build_indexable_records(
                &parsed,
                artifact_id,
                blob_id,
                &request.source_path,
                source_hash,
            ) {
                Ok(records) => records,
                Err(error) => {
                    tracing::error!(
                        artifact_id = %artifact_id.value(),
                        %error,
                        "parser emitted malformed indexable records"
                    );
                    return false;
                }
            }
        } else {
            if !parsed.chunks.is_empty() || !parsed.cards.is_empty() {
                tracing::error!(
                    artifact_id = %artifact_id.value(),
                    "non-indexable parser result contains indexable records"
                );
                return false;
            }
            (Vec::new(), Vec::new(), Vec::new())
        };
        let tree_nodes = parsed.tree.nodes().to_vec();
        if Self::send_input_blocking(
            &self.input_tx,
            DomainInput::ParserCompleted(ParserResult {
                artifact_id: parsed.artifact_id,
                artifact_version_id: parsed.artifact_version_id,
                content_hash: parsed.content_hash,
                status,
                tree_root_id: Some(parsed.tree.root_id()),
                tree_nodes,
                chunks,
                cards,
            }),
            "parser completion",
        )
        .await
        .is_err()
        {
            return false;
        }
        if !indexable {
            return true;
        }
        for evidence in evidence_inputs {
            if Self::send_input_blocking(
                &self.input_tx,
                DomainInput::RecordEvidence(evidence),
                "record evidence",
            )
            .await
            .is_err()
            {
                return false;
            }
        }
        Self::send_input_blocking(
            &self.input_tx,
            DomainInput::StartFullTextIndex(maestria_domain::StartFullTextIndex {
                artifact_id: parsed.artifact_id,
            }),
            "start full-text index",
        )
        .await
        .is_ok()
    }
}
