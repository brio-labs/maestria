use crate::types::*;

impl KernelState {
    // ── Single-event dispatch helpers ──────────────────────────────────────

    pub(super) fn process_register_artifact(
        &mut self,
        input: RegisterArtifactInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_register_artifact(input)?;
        output.events.push(event.clone());
        output
            .effects
            .push(MaestriaEffect::PersistEvent { envelope: event });
        Ok(output)
    }

    pub(super) fn process_register_chunk(
        &mut self,
        input: RegisterChunkInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_register_chunk(input.clone())?;
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: event.clone(),
        });
        if let DomainEvent::ChunkRegistered {
            artifact_id,
            chunk_id,
            ..
        } = event.event
        {
            output
                .effects
                .push(MaestriaEffect::IndexFullText(IndexFullTextRequest {
                    artifact_id,
                    chunk_id,
                }));
            output
                .effects
                .push(MaestriaEffect::IndexVector(IndexVectorRequest {
                    artifact_id,
                    chunk_id,
                }));
        }
        Ok(output)
    }

    pub(super) fn process_create_card(
        &mut self,
        input: CreateCardInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_create_card(input.clone())?;
        output.events.push(event.clone());
        output
            .effects
            .push(MaestriaEffect::PersistEvent { envelope: event });
        Ok(output)
    }

    pub(super) fn process_record_evidence(
        &mut self,
        input: RecordEvidenceInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let maybe_event = self.handle_record_evidence(input.clone())?;
        if let Some(event) = maybe_event {
            output.events.push(event.clone());
            output.effects.push(MaestriaEffect::PersistEvent {
                envelope: event.clone(),
            });
            let claim_id = match event.event {
                DomainEvent::EvidenceRecorded { claim_id, .. } => claim_id,
                _ => None,
            };
            if let Some(claim_id) = claim_id {
                output
                    .effects
                    .push(MaestriaEffect::RunValidation(RunValidationRequest {
                        task_id: None,
                        claim_id: Some(claim_id),
                        validation_report_id: ValidationReportId::new(0),
                    }));
            }
        }
        Ok(output)
    }

    pub(super) fn process_create_claim(
        &mut self,
        input: CreateClaimInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_create_claim(input.clone())?;
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: event.clone(),
        });
        output
            .effects
            .push(MaestriaEffect::RunValidation(RunValidationRequest {
                task_id: None,
                claim_id: Some(input.claim_id),
                validation_report_id: ValidationReportId::new(0),
            }));
        Ok(output)
    }

    pub(super) fn process_open_task(
        &mut self,
        input: OpenTaskInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_open_task(input.clone())?;
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: event.clone(),
        });
        if input.priority == TaskPriority::High {
            let task_id = match event.event {
                DomainEvent::TaskOpened { task_id, .. } => task_id,
                _ => input.task_id,
            };
            output
                .effects
                .push(MaestriaEffect::RequestApproval(RequestApprovalRequest {
                    task_id,
                }));
        }
        Ok(output)
    }

    pub(super) fn process_change_task_status(
        &mut self,
        input: ChangeTaskStatusInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let (from, to) = self.handle_change_task_status(input.task_id, input.to)?;
        let event = self.emit_event(DomainEvent::TaskStatusChanged {
            task_id: input.task_id,
            from,
            to,
        });
        output.events.push(event.clone());
        output
            .effects
            .push(MaestriaEffect::PersistEvent { envelope: event });
        Ok(output)
    }

    pub(super) fn process_complete_task(
        &mut self,
        input: CompleteTaskInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_complete_task(input)?;
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: event.clone(),
        });
        output
            .effects
            .push(MaestriaEffect::PersistState(PersistStateRequest {
                reason: "validated task completion".to_string(),
            }));
        Ok(output)
    }

    pub(super) fn process_link_evidence_to_claim(
        &mut self,
        input: LinkEvidenceToClaimInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let claim_id = input.claim_id;
        let event = self.handle_link_evidence_to_claim(input.clone())?;
        output.events.push(event.clone());
        output
            .effects
            .push(MaestriaEffect::PersistEvent { envelope: event });
        output
            .effects
            .push(MaestriaEffect::RunValidation(RunValidationRequest {
                task_id: None,
                claim_id: Some(claim_id),
                validation_report_id: ValidationReportId::new(0),
            }));
        Ok(output)
    }

    pub(super) fn process_link_evidence_to_task(
        &mut self,
        input: LinkEvidenceToTaskInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        if let Some(event) = self.handle_link_evidence_to_task(input)? {
            output.events.push(event.clone());
            output
                .effects
                .push(MaestriaEffect::PersistEvent { envelope: event });
        }
        Ok(output)
    }

    pub(super) fn process_create_relation(
        &mut self,
        input: CreateRelationInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_create_relation(input)?;
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: event.clone(),
        });
        if let DomainEvent::RelationCreated { relation_id, .. } = event.event {
            output
                .effects
                .push(MaestriaEffect::UpdateGraph(UpdateGraphRequest {
                    relation_id,
                }));
        }
        Ok(output)
    }

    pub(super) fn process_create_memory_candidate(
        &mut self,
        input: CreateMemoryCandidateInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_create_memory_candidate(input)?;
        output.events.push(event.clone());
        output
            .effects
            .push(MaestriaEffect::PersistEvent { envelope: event });
        Ok(output)
    }

    pub(super) fn process_propose_memory_candidate(
        &mut self,
        input: ProposeMemoryCandidateInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let envelopes = self.handle_propose_memory_candidate(input)?;
        for envelope in &envelopes {
            output.events.push(envelope.clone());
            output.effects.push(MaestriaEffect::PersistEvent {
                envelope: envelope.clone(),
            });
        }
        Ok(output)
    }

    pub(super) fn process_promote_memory(
        &mut self,
        input: PromoteMemoryInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_promote_memory(input)?;
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: event.clone(),
        });
        Ok(output)
    }

    pub(super) fn process_contradict_memory(
        &mut self,
        input: ContradictMemoryInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_contradict_memory(input)?;
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: event.clone(),
        });
        Ok(output)
    }

    pub(super) fn process_deprecate_memory(
        &mut self,
        input: DeprecateMemoryInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_deprecate_memory(input)?;
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: event.clone(),
        });
        Ok(output)
    }

    pub(super) fn process_supersede_memory(
        &mut self,
        input: SupersedeMemoryInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_supersede_memory(input)?;
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: event.clone(),
        });
        Ok(output)
    }

    pub(super) fn process_record_validation_report(
        &mut self,
        input: RecordValidationReportInput,
    ) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();
        let event = self.handle_record_validation_report(input)?;
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: event.clone(),
        });
        Ok(output)
    }
}
