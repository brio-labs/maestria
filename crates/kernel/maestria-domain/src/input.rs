use crate::types::*;
mod handlers;

impl KernelState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_input(&mut self, input: DomainInput) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();

        match input {
            DomainInput::RegisterArtifact(input) => {
                let event = self.handle_register_artifact(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    envelope: event,
                });
            }
            DomainInput::RegisterChunk(input) => {
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
            }
            DomainInput::CreateCard(input) => {
                let event = self.handle_create_card(input.clone())?;
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { envelope: event });
            }
            DomainInput::RecordEvidence(input) => {
                let event = self.handle_record_evidence(input.clone())?;
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
            DomainInput::CreateClaim(input) => {
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
            }
            DomainInput::OpenTask(input) => {
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
            }
            DomainInput::ChangeTaskStatus(input) => {
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
            }
            DomainInput::CompleteTask(input) => {
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
            }
            DomainInput::LinkEvidenceToClaim(input) => {
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
            }
            DomainInput::CreateRelation(input) => {
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
            }
            DomainInput::CreateMemoryCandidate(input) => {
                let event = self.handle_create_memory_candidate(input)?;
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { envelope: event });
            }
            DomainInput::PromoteMemory(input) => {
                let event = self.handle_promote_memory(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    envelope: event.clone(),
                });
            }
            DomainInput::ContradictMemory(input) => {
                let event = self.handle_contradict_memory(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    envelope: event.clone(),
                });
            }
            DomainInput::DeprecateMemory(input) => {
                let event = self.handle_deprecate_memory(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    envelope: event.clone(),
                });
            }
            DomainInput::SupersedeMemory(input) => {
                let event = self.handle_supersede_memory(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    envelope: event.clone(),
                });
            }
            DomainInput::RecordValidationReport(input) => {
                let event = self.handle_record_validation_report(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    envelope: event.clone(),
                });
            }
            DomainInput::UserIntent(input) => {
                let event = self.handle_user_intent(input.clone())?;
                for entry in event {
                    output.events.push(entry.clone());
                    output
                        .effects
                        .push(MaestriaEffect::PersistEvent { envelope: entry });
                }
            }
            DomainInput::ArtifactDetected(input) => {
                if self.artifacts.contains_key(&input.artifact_id) {
                    // Already preflighted — no events/effects for unchanged content
                } else {
                    let event = self.handle_register_artifact(RegisterArtifactInput {
                        artifact_id: input.artifact_id,
                        title: input.title,
                    })?;
                    let blob = input.source_bytes.clone();
                    output.events.push(event.clone());
                    output.effects.push(MaestriaEffect::PersistEvent {
                        envelope: event,
                    });
                    output.effects.push(MaestriaEffect::StoreBlob(
                        StoreBlobRequest {
                            artifact_id: input.artifact_id,
                            payload: blob.clone(),
                        },
                    ));
                    output.effects.push(MaestriaEffect::ParseArtifact(
                        ParseArtifactRequest {
                            artifact_id: input.artifact_id,
                            source_path: input.source_path,
                            source_bytes: blob,
                        },
                    ));
                }
            }
            DomainInput::ParserCompleted(input) => {
                let generated = self.handle_parser_completed(input)?;
                for envelope in generated {
                    output.events.push(envelope.clone());
                    output
                        .effects
                        .push(MaestriaEffect::PersistEvent { envelope });
                }
            }
            DomainInput::SearchCompleted(input) => {
                let generated = self.handle_search_completed(input)?;
                for envelope in generated {
                    output.events.push(envelope.clone());
                    output
                        .effects
                        .push(MaestriaEffect::PersistEvent { envelope });
                }
            }
            DomainInput::HarnessRunCompleted(input) => {
                let generated = self.handle_harness_completed(input)?;
                for envelope in generated {
                    output.events.push(envelope.clone());
                    output
                        .effects
                        .push(MaestriaEffect::PersistEvent { envelope });
                }
            }
            DomainInput::ValidationCompleted(input) => {
                let event = self.handle_validation_completed(input)?;
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { envelope: event });
            }
            DomainInput::ApprovalResolved(input) => {
                let envelopes = self.handle_approval_resolved(input)?;
                for envelope in envelopes {
                    output.events.push(envelope.clone());
                    output
                        .effects
                        .push(MaestriaEffect::PersistEvent { envelope });
                }
            }
            DomainInput::ClockTick(tick) => {
                let event = self.emit_event(DomainEvent::TickObserved { at: tick });
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { envelope: event });
            }
        }

        Ok(output)
    }

    fn emit_event(&mut self, event: DomainEvent) -> DomainEventEnvelope {
        let id = EventId(self.event_log.len() as u64 + 1);
        let sequence = SequenceNumber(id.value());
        let envelope = DomainEventEnvelope {
            id,
            sequence,
            event,
        };
        self.event_log.push(envelope.clone());
        envelope
    }
}
