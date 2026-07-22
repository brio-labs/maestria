use crate::types::*;

impl KernelState {
    // ── Output helpers ─────────────────────────────────────────────────────

    /// Build a [`KernelOutput`] containing a single event and its persist effect.
    pub(crate) fn output_for_event(event: DomainEventEnvelope) -> KernelOutput {
        let mut output = KernelOutput::default();
        output.events.push(event.clone());
        output.effects.push(MaestriaEffect::PersistEvent {
            envelope: Box::new(event),
        });
        output
    }

    /// Build a [`KernelOutput`] containing multiple events and their persist effects.
    pub(crate) fn output_for_events(events: Vec<DomainEventEnvelope>) -> KernelOutput {
        let mut output = KernelOutput::default();
        for event in events {
            output.events.push(event.clone());
            output.effects.push(MaestriaEffect::PersistEvent {
                envelope: Box::new(event),
            });
        }
        output
    }

    // ── Single-event dispatch helpers ──────────────────────────────────────

    pub(super) fn process_register_artifact(
        &mut self,
        input: RegisterArtifactInput,
    ) -> Result<KernelOutput, DomainError> {
        let event = self.handle_register_artifact(input)?;
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_register_chunk(
        &mut self,
        input: RegisterChunkInput,
    ) -> Result<KernelOutput, DomainError> {
        let event = self.handle_register_chunk(input)?;
        let ids = match &event.event {
            DomainEvent::ChunkRegistered {
                artifact_id,
                chunk_id,
                ..
            } => Some((*artifact_id, *chunk_id)),
            _ => None,
        };
        let mut output = Self::output_for_event(event);
        if let Some((artifact_id, chunk_id)) = ids {
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
        let event = self.handle_create_card(input)?;
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_record_evidence(
        &mut self,
        input: RecordEvidenceInput,
    ) -> Result<KernelOutput, DomainError> {
        let maybe_event = self.handle_record_evidence(input)?;
        let Some(event) = maybe_event else {
            return Ok(KernelOutput::default());
        };
        let claim_id = match &event.event {
            DomainEvent::EvidenceRecorded { claim_id, .. } => *claim_id,
            _ => None,
        };
        let mut output = Self::output_for_event(event);
        if let Some(claim_id) = claim_id {
            output
                .effects
                .push(MaestriaEffect::RunValidation(RunValidationRequest {
                    task_id: None,
                    claim_id: Some(claim_id),
                    validation_report_id: ValidationReportId::new(0),
                }));
        }
        Ok(output)
    }

    pub(super) fn process_create_claim(
        &mut self,
        input: CreateClaimInput,
    ) -> Result<KernelOutput, DomainError> {
        let claim_id = input.claim_id;
        let event = self.handle_create_claim(input)?;
        let mut output = Self::output_for_event(event);
        output
            .effects
            .push(MaestriaEffect::RunValidation(RunValidationRequest {
                task_id: None,
                claim_id: Some(claim_id),
                validation_report_id: ValidationReportId::new(0),
            }));
        Ok(output)
    }

    pub(super) fn process_open_task(
        &mut self,
        input: OpenTaskInput,
    ) -> Result<KernelOutput, DomainError> {
        let priority = input.priority;
        let fallback_task_id = input.task_id;
        let event = self.handle_open_task(input)?;
        let task_id = match &event.event {
            DomainEvent::TaskOpened { task_id, .. } => *task_id,
            _ => fallback_task_id,
        };
        let mut output = Self::output_for_event(event);
        if priority == TaskPriority::High {
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
        let (from, to) = self.handle_change_task_status(input.task_id, input.to)?;
        let event = self.emit_event(DomainEvent::TaskStatusChanged {
            task_id: input.task_id,
            from,
            to,
        });
        let mut output = Self::output_for_event(event);
        if input.to == TaskStatus::Validating {
            output
                .effects
                .push(MaestriaEffect::RunValidation(RunValidationRequest {
                    task_id: Some(input.task_id),
                    claim_id: None,
                    validation_report_id: ValidationReportId::new(0),
                }));
        }
        Ok(output)
    }

    pub(super) fn process_complete_task(
        &mut self,
        input: CompleteTaskInput,
    ) -> Result<KernelOutput, DomainError> {
        let event = self.handle_complete_task(input)?;
        let mut output = Self::output_for_event(event);
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
        let claim_id = input.claim_id;
        let event = self.handle_link_evidence_to_claim(input)?;
        let mut output = Self::output_for_event(event);
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
        if let Some(event) = self.handle_link_evidence_to_task(input)? {
            Ok(Self::output_for_event(event))
        } else {
            Ok(KernelOutput::default())
        }
    }

    pub(super) fn process_create_relation(
        &mut self,
        input: CreateRelationInput,
    ) -> Result<KernelOutput, DomainError> {
        let event = self.handle_create_relation(input)?;
        let relation_id = match &event.event {
            DomainEvent::RelationCreated {
                relation_id,
                evidence_id: Some(_),
                ..
            } => Some(*relation_id),
            _ => None,
        };
        let mut output = Self::output_for_event(event);
        if let Some(relation_id) = relation_id {
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
        let event = self.handle_create_memory_candidate(input)?;
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_propose_memory_candidate(
        &mut self,
        input: ProposeMemoryCandidateInput,
    ) -> Result<KernelOutput, DomainError> {
        let envelopes = self.handle_propose_memory_candidate(input)?;
        Ok(Self::output_for_events(envelopes))
    }

    pub(super) fn process_promote_memory(
        &mut self,
        input: PromoteMemoryInput,
    ) -> Result<KernelOutput, DomainError> {
        let event = self.handle_promote_memory(input)?;
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_contradict_memory(
        &mut self,
        input: ContradictMemoryInput,
    ) -> Result<KernelOutput, DomainError> {
        let event = self.handle_contradict_memory(input)?;
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_deprecate_memory(
        &mut self,
        input: DeprecateMemoryInput,
    ) -> Result<KernelOutput, DomainError> {
        let event = self.handle_deprecate_memory(input)?;
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_supersede_memory(
        &mut self,
        input: SupersedeMemoryInput,
    ) -> Result<KernelOutput, DomainError> {
        let event = self.handle_supersede_memory(input)?;
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_record_validation_report(
        &mut self,
        input: RecordValidationReportInput,
    ) -> Result<KernelOutput, DomainError> {
        let event = self.handle_record_validation_report(input)?;
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_request_task_validation(
        &mut self,
        input: RequestTaskValidation,
    ) -> Result<KernelOutput, DomainError> {
        if !self.tasks.contains_key(&input.task_id) {
            return Err(DomainError::MissingTask { id: input.task_id });
        }
        let mut output = KernelOutput::default();
        output
            .effects
            .push(MaestriaEffect::RunValidation(RunValidationRequest {
                task_id: Some(input.task_id),
                claim_id: None,
                validation_report_id: ValidationReportId::new(0),
            }));
        Ok(output)
    }
}
