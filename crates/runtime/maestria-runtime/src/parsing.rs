use crate::config::EffectExecutionContext;
use crate::persistence::wait_for_parser_started_persistence;
use maestria_domain::{
    Artifact, ArtifactId, BlobId, ContentRange, DomainInput, EvidenceKind, LogicalTick,
    ParseArtifactRequest, ParserResult, ParserStarted, RecordEvidenceInput, RegisterChunkInput,
    StartFullTextIndex, content_hash, evidence_id_for, excerpt_for,
};
use maestria_ports::{FileHandle, FileMetadata, ParseContext, SourceSpan};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

impl EffectExecutionContext {
    /// Parse an artifact into chunks, cards, and evidence.
    /// Handles both fresh ingestion (blob storage + ParserStarted event)
    /// and resume (blob already stored, ParserStarted already persisted).
    pub(crate) async fn handle_parse_artifact(
        &self,
        request: ParseArtifactRequest,
        persistence_barrier_timeout: Option<Duration>,
    ) -> bool {
        // 1. Resolve the artifact (repo → state → ephemeral).
        let Some(artifact) = self
            .resolve_artifact_for_parse(request.artifact_id, &request.source_path)
            .await
        else {
            return false;
        };

        // 2. Resolve the bytes to parse and the blob identity.
        let Ok((parse_bytes, blob_id, is_resume)) =
            self.resolve_blob_for_parse(&request, artifact.id)
        else {
            return false;
        };

        let path = PathBuf::from(&request.source_path);

        // 3. Check that the parser supports this file type.
        if !self.check_parser_support(&path, &parse_bytes, artifact.id) {
            return false;
        }

        let source_hash = content_hash(&parse_bytes);

        // 4. On fresh ingestion, publish the durable ParserStarted marker and
        //    wait for it to become observable in the event log (persistence barrier).
        if !is_resume
            && !self
                .publish_parser_started(
                    artifact.id,
                    &artifact.title,
                    &request.source_path,
                    &source_hash,
                    blob_id,
                    persistence_barrier_timeout,
                )
                .await
        {
            return false;
        }

        // 5. Run the parser and emit domain inputs for the results.
        self.parse_and_emit(
            &request,
            artifact.id,
            parse_bytes,
            blob_id,
            source_hash,
            path,
        )
        .await
    }

    /// Resolve the artifact for parsing: try the repository, then in-memory state,
    /// then fall back to an ephemeral artifact for staged/resume ingestion.
    async fn resolve_artifact_for_parse(
        &self,
        artifact_id: ArtifactId,
        source_path: &str,
    ) -> Option<Artifact> {
        match self.adapters.artifact_repo.get(artifact_id) {
            Ok(Some(artifact)) => Some(artifact),
            Ok(None) => {
                let state_read = self.state.read().await;
                if let Some(artifact) = state_read.artifacts.get(&artifact_id).cloned() {
                    Some(artifact)
                } else {
                    // Staged ingestion or resume: no persisted artifact yet. Construct an
                    // ephemeral typed parse context so the parser can proceed with the
                    // request metadata. The artifact is committed later by the domain
                    // handler when it receives ParserCompleted.
                    tracing::debug!(
                        artifact_id = %artifact_id,
                        "no persisted artifact; constructing ephemeral context for parse"
                    );
                    Some(Artifact {
                        id: artifact_id,
                        title: source_path.to_owned(),
                        chunk_ids: BTreeSet::new(),
                        card_ids: BTreeSet::new(),
                        claim_ids: BTreeSet::new(),
                        evidence_ids: BTreeSet::new(),
                        index_status: maestria_domain::IndexStatus::default(),
                        content_hash: None,
                    })
                }
            }
            Err(error) => {
                tracing::error!(artifact_id = %artifact_id, %error, "failed to load artifact for parse");
                None
            }
        }
    }

    /// Resolve the bytes to parse and the blob identity.
    /// - Fresh ingestion (`source_blob` is `None`): store bytes in the blob store
    ///   and obtain an immutable `BlobId`.
    /// - Resume (`source_blob` is `Some`): fetch the exact bytes from the blob store.
    fn resolve_blob_for_parse(
        &self,
        request: &ParseArtifactRequest,
        artifact_id: ArtifactId,
    ) -> Result<(Vec<u8>, BlobId, bool), ()> {
        if let Some(blob_id) = request.source_blob {
            match self.adapters.blob_store.get(blob_id) {
                Ok(bytes) => Ok((bytes, blob_id, true)),
                Err(error) => {
                    tracing::error!(
                        artifact_id = %artifact_id,
                        %blob_id,
                        %error,
                        "resume blob missing from store"
                    );
                    Err(())
                }
            }
        } else {
            match self.adapters.blob_store.put(request.source_bytes.clone()) {
                Ok(blob_id) => Ok((request.source_bytes.clone(), blob_id, false)),
                Err(error) => {
                    tracing::error!(
                        artifact_id = %artifact_id,
                        %error,
                        "failed to store source blob"
                    );
                    Err(())
                }
            }
        }
    }

    /// Build a `FileMetadata` from the path and bytes, then check whether
    /// the configured parser supports the file.
    fn check_parser_support(
        &self,
        path: &Path,
        parse_bytes: &[u8],
        artifact_id: ArtifactId,
    ) -> bool {
        let metadata = FileMetadata {
            path: path.to_path_buf(),
            size: parse_bytes.len(),
            extension: path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_owned),
        };
        if !self.adapters.parser.supports(&metadata) {
            tracing::warn!(
                artifact_id = %artifact_id,
                parser = self.adapters.parser.id(),
                path = %metadata.path.display(),
                "parser does not support artifact"
            );
            return false;
        }
        true
    }

    /// Send `ParserStarted` and (when a barrier timeout is configured) block
    /// until the event is observable in the event log. Returns `false` if the
    /// barrier times out or a scan errors.
    async fn publish_parser_started(
        &self,
        artifact_id: ArtifactId,
        artifact_title: &str,
        source_path: &str,
        source_hash: &str,
        blob_id: BlobId,
        barrier_timeout: Option<Duration>,
    ) -> bool {
        let title = {
            let state_read = self.state.read().await;
            state_read
                .pending_artifacts
                .get(&artifact_id)
                .map_or_else(|| artifact_title.to_owned(), |p| p.title.clone())
        };
        if Self::send_input(
            &self.input_tx,
            DomainInput::ParserStarted(ParserStarted {
                artifact_id,
                title,
                source_path: source_path.to_owned(),
                content_hash: source_hash.to_owned(),
                blob_id,
            }),
            "parser started",
        )
        .is_err()
        {
            return false;
        }

        // Persistence barrier: wait until the ParserStarted event is
        // observable in the event log before proceeding to parse. This
        // closes the crash window where the parser could start before
        // the durable resume marker is committed.
        // Only active when the runtime path supplies a timeout (production);
        // direct unit-test calls skip this via None.
        if let Some(barrier_timeout) = barrier_timeout {
            let capped = barrier_timeout.min(Duration::from_secs(30));
            let persisted = wait_for_parser_started_persistence(
                &*self.adapters.event_log,
                artifact_id,
                blob_id,
                source_hash,
                capped,
            )
            .await;
            if !persisted {
                tracing::error!(
                    artifact_id = %artifact_id,
                    "ParserStarted persistence barrier failed; not parsing"
                );
                return false;
            }
        }
        true
    }

    /// Run the parser and emit the resulting domain inputs:
    /// `ParserCompleted`, `RecordEvidence` per chunk, and `StartFullTextIndex`.
    async fn parse_and_emit(
        &self,
        request: &ParseArtifactRequest,
        artifact_id: ArtifactId,
        parse_bytes: Vec<u8>,
        blob_id: BlobId,
        source_hash: String,
        path: PathBuf,
    ) -> bool {
        let file = FileHandle {
            path,
            bytes: parse_bytes,
        };
        match self
            .adapters
            .parser
            .parse(file, ParseContext { artifact_id })
        {
            Ok(parsed) => {
                let observed_at = LogicalTick::new(1);

                // Collect evidence inputs and chunk registrations in a single pass.
                let mut evidence_inputs: Vec<RecordEvidenceInput> = Vec::new();
                let mut chunks: Vec<RegisterChunkInput> = Vec::new();
                for (order, chunk) in parsed.chunks.iter().enumerate() {
                    let evidence_id = evidence_id_for(artifact_id, order as u32);
                    let excerpt = excerpt_for(&chunk.text);
                    let kind = match &chunk.source_span {
                        SourceSpan::TextSpan {
                            start_line,
                            end_line,
                        } => EvidenceKind::FileSpan {
                            path: request.source_path.clone(),
                            range: ContentRange {
                                start: *start_line,
                                end: *end_line,
                            },
                            content_hash: source_hash.clone(),
                            snapshot: Some(blob_id),
                        },
                        SourceSpan::PdfSpan { page } => EvidenceKind::PdfSpan {
                            blob: blob_id,
                            page_start: *page as u32,
                            page_end: *page as u32,
                        },
                    };
                    evidence_inputs.push(RecordEvidenceInput {
                        evidence_id,
                        artifact_id,
                        claim_id: None,
                        kind,
                        excerpt,
                        observed_at,
                    });
                    chunks.push(RegisterChunkInput {
                        chunk_id: chunk.chunk_id,
                        artifact_id: chunk.artifact_id,
                        order: (order.min(u32::MAX as usize)) as u32,
                        text: chunk.text.clone(),
                    });
                }
                // Send ParserCompleted first so the domain handler commits the artifact
                // (including ArtifactRegistered / PendingIndex) before evidence arrives.
                // Evidence inputs follow after the artifact is guaranteed to exist in
                // domain state. After evidence is persisted, StartFullTextIndex triggers
                // full-text indexing via the domain's deferred gate.
                if Self::send_input(
                    &self.input_tx,
                    DomainInput::ParserCompleted(ParserResult {
                        artifact_id: parsed.artifact_id,
                        chunks,
                        cards: parsed.cards,
                    }),
                    "parser completion",
                )
                .is_err()
                {
                    return false;
                }

                for evidence in evidence_inputs {
                    if Self::send_input(
                        &self.input_tx,
                        DomainInput::RecordEvidence(evidence),
                        "record evidence",
                    )
                    .is_err()
                    {
                        return false;
                    }
                }

                if Self::send_input(
                    &self.input_tx,
                    DomainInput::StartFullTextIndex(StartFullTextIndex {
                        artifact_id: parsed.artifact_id,
                    }),
                    "start full-text index",
                )
                .is_err()
                {
                    return false;
                }
            }
            Err(error) => {
                tracing::error!(artifact_id = %artifact_id, %error, "parser failed");
                return false;
            }
        }

        true
    }
}
