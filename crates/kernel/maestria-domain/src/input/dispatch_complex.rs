use crate::types::*;

impl KernelState {
    // ── Multi-event dispatch helpers ───────────────────────────────────────

    pub(super) fn process_user_intent(
        &mut self,
        input: UserIntent,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_user_intent(input.clone())?;
        for entry in event {
            output.events.push(entry.clone());
            output
                .effects
                .push(MaestriaEffect::PersistEvent { envelope: entry });
        }
        if input.priority == TaskPriority::High {
            output
                .effects
                .push(MaestriaEffect::RequestApproval(RequestApprovalRequest {
                    task_id: input.task_id,
                }));
        }
        Ok(output)
    }

    pub(super) fn process_artifact_detected(
        &mut self,
        input: ArtifactDetected,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let existing = self.artifacts.get(&input.artifact_id);
        let unchanged = existing.is_some_and(|a| {
            a.content_hash.as_deref() == Some(&input.content_hash)
                && a.index_status == IndexStatus::Indexed
        });

        // Also guard against duplicate detections while a parser is
        // already in-flight with the same content_hash — avoids
        // redundant ParseArtifact effects.
        let pending_unchanged = self
            .pending_parsers
            .get(&input.artifact_id)
            .is_some_and(|p| p.content_hash == input.content_hash);

        if unchanged || pending_unchanged {
            // Equal indexed hash or identical pending parse — terminal no-op
        } else {
            // Store pending metadata in-memory only; no persisted events yet.
            // The artifact is committed only on successful ParserCompleted.
            self.pending_artifacts.insert(
                input.artifact_id,
                PendingArtifact {
                    artifact_id: input.artifact_id,
                    title: input.title.clone(),
                    content_hash: input.content_hash.clone(),
                },
            );

            output
                .effects
                .push(MaestriaEffect::ParseArtifact(ParseArtifactRequest {
                    artifact_id: input.artifact_id,
                    source_path: input.source_path,
                    source_bytes: input.source_bytes,
                    source_blob: None,
                }));
        }
        Ok(output)
    }

    pub(super) fn process_parser_completed(
        &mut self,
        input: ParserResult,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let generated = self.handle_parser_completed(input)?;
        for envelope in generated {
            output.events.push(envelope.clone());
            output
                .effects
                .push(MaestriaEffect::PersistEvent { envelope });
        }
        Ok(output)
    }

    pub(super) fn process_full_text_index_completed(
        &mut self,
        input: FullTextIndexCompleted,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let generated = self.handle_full_text_index_completed(input)?;
        for envelope in generated {
            output.events.push(envelope.clone());
            output
                .effects
                .push(MaestriaEffect::PersistEvent { envelope });
        }
        Ok(output)
    }

    pub(super) fn process_start_full_text_index(
        &mut self,
        input: StartFullTextIndex,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let generated = self.handle_start_full_text_index(&input)?;
        // If the handler terminalized (crash-after-evidence recovery),
        // push the ArtifactIndexed event before checking pending chunks.
        for envelope in generated {
            output.events.push(envelope.clone());
            output
                .effects
                .push(MaestriaEffect::PersistEvent { envelope });
        }
        // Only emit IndexFullText effects for chunks still pending.
        for chunk in self.chunks.values() {
            if chunk.artifact_id == input.artifact_id && self.pending_full_text.contains(&chunk.id)
            {
                output
                    .effects
                    .push(MaestriaEffect::IndexFullText(IndexFullTextRequest {
                        artifact_id: input.artifact_id,
                        chunk_id: chunk.id,
                    }));
            }
        }
        Ok(output)
    }

    pub(super) fn process_search_completed(
        &mut self,
        input: SearchResultSet,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let generated = self.handle_search_completed(input)?;
        for envelope in generated {
            output.events.push(envelope.clone());
            output
                .effects
                .push(MaestriaEffect::PersistEvent { envelope });
        }
        Ok(output)
    }

    pub(super) fn process_harness_run_completed(
        &mut self,
        input: HarnessRunCompleted,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let generated = self.handle_harness_completed(input)?;
        for envelope in generated {
            output.events.push(envelope.clone());
            output
                .effects
                .push(MaestriaEffect::PersistEvent { envelope });
        }
        Ok(output)
    }

    pub(super) fn process_validation_completed(
        &mut self,
        input: ValidationCompleted,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_validation_completed(input)?;
        output.events.push(event.clone());
        output
            .effects
            .push(MaestriaEffect::PersistEvent { envelope: event });
        Ok(output)
    }

    pub(super) fn process_approval_resolved(
        &mut self,
        input: ApprovalDecision,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let envelopes = self.handle_approval_resolved(input)?;
        for envelope in envelopes {
            output.events.push(envelope.clone());
            output
                .effects
                .push(MaestriaEffect::PersistEvent { envelope });
        }
        Ok(output)
    }

    pub(super) fn process_parser_started(
        &mut self,
        input: ParserStarted,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        // Idempotent: identical metadata must not emit duplicate events.
        if let Some(existing) = self.pending_parsers.get(&input.artifact_id)
            && existing.title == input.title
            && existing.source_path == input.source_path
            && existing.content_hash == input.content_hash
            && existing.blob_id == input.blob_id
        {
            // Identical metadata — skip duplicate event and effect.
            return Ok(output);
        }
        // Record durable pending-parser metadata; emitted as a PersistEvent
        // so that restart can find this artifact if parsing never finishes.
        self.pending_parsers
            .insert(input.artifact_id, input.clone());
        let event = self.emit_event(DomainEvent::ParserStarted {
            artifact_id: input.artifact_id,
            title: input.title,
            source_path: input.source_path,
            content_hash: input.content_hash,
            blob_id: input.blob_id,
        });
        output.events.push(event.clone());
        output
            .effects
            .push(MaestriaEffect::PersistEvent { envelope: event });
        Ok(output)
    }

    pub(super) fn process_resume_parser(
        &mut self,
        input: ParserStarted,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        // Idempotent re-drive: check that the pending entry exists (it must
        // have been reconstructed from replay), then emit a ParseArtifact
        // effect with the existing blob so the runtime re-parses from the
        // stored bytes.
        if !self.pending_parsers.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        output
            .effects
            .push(MaestriaEffect::ParseArtifact(ParseArtifactRequest {
                artifact_id: input.artifact_id,
                source_path: input.source_path,
                source_bytes: Vec::new(),
                source_blob: Some(input.blob_id),
            }));
        Ok(output)
    }

    pub(super) fn process_clock_tick(
        &mut self,
        tick: LogicalTick,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.emit_event(DomainEvent::TickObserved { at: tick });
        output.events.push(event.clone());
        output
            .effects
            .push(MaestriaEffect::PersistEvent { envelope: event });
        Ok(output)
    }

    pub(super) fn process_search_executed(
        &mut self,
        input: SearchExecutedInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let envelope = self.handle_search_executed(input)?;
        output.events.push(envelope.clone());
        output
            .effects
            .push(MaestriaEffect::PersistEvent { envelope });
        Ok(output)
    }
}
