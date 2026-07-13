use crate::types::*;
mod artifact;
mod card;
mod claim;
mod dispatch;
mod dispatch_complex;
mod dispatch_crud;
mod evidence;
mod handlers;
mod index;
mod memory;
mod orchestration;
mod relation;
mod task;
mod validation;

impl KernelState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_input(&mut self, input: DomainInput) -> Result<KernelOutput, DomainError> {
        match input {
            DomainInput::RegisterArtifact(input) => self.process_register_artifact(input),
            DomainInput::RegisterChunk(input) => self.process_register_chunk(input),
            DomainInput::CreateCard(input) => self.process_create_card(input),
            DomainInput::RecordEvidence(input) => self.process_record_evidence(input),
            DomainInput::CreateClaim(input) => self.process_create_claim(input),
            DomainInput::OpenTask(input) => self.process_open_task(input),
            DomainInput::ChangeTaskStatus(input) => self.process_change_task_status(input),
            DomainInput::CompleteTask(input) => self.process_complete_task(input),
            DomainInput::LinkEvidenceToTask(input) => self.process_link_evidence_to_task(input),
            DomainInput::LinkEvidenceToClaim(input) => self.process_link_evidence_to_claim(input),
            DomainInput::CreateRelation(input) => self.process_create_relation(input),
            DomainInput::CreateMemoryCandidate(input) => {
                self.process_create_memory_candidate(input)
            }
            DomainInput::PromoteMemory(input) => self.process_promote_memory(input),
            DomainInput::ContradictMemory(input) => self.process_contradict_memory(input),
            DomainInput::DeprecateMemory(input) => self.process_deprecate_memory(input),
            DomainInput::SupersedeMemory(input) => self.process_supersede_memory(input),
            DomainInput::RecordValidationReport(input) => {
                self.process_record_validation_report(input)
            }
            DomainInput::UserIntent(input) => self.process_user_intent(input),
            DomainInput::ArtifactDetected(input) => self.process_artifact_detected(input),
            DomainInput::ParserCompleted(input) => self.process_parser_completed(input),
            DomainInput::FullTextIndexCompleted(input) => {
                self.process_full_text_index_completed(input)
            }
            DomainInput::StartFullTextIndex(input) => self.process_start_full_text_index(input),
            DomainInput::SearchCompleted(input) => self.process_search_completed(input),
            DomainInput::HarnessRunCompleted(input) => self.process_harness_run_completed(input),
            DomainInput::ValidationCompleted(input) => self.process_validation_completed(input),
            DomainInput::ApprovalResolved(input) => self.process_approval_resolved(input),
            DomainInput::ParserStarted(input) => self.process_parser_started(input),
            DomainInput::ResumeParser(input) => self.process_resume_parser(input),
            DomainInput::ClockTick(tick) => self.process_clock_tick(tick),
        }
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
