use crate::types::*;

impl KernelState {
    pub(crate) fn process_start_index_generation(
        &mut self,
        input: StartIndexGenerationInput,
    ) -> Result<KernelOutput, DomainError> {
        let event = DomainEvent::IndexGenerationStarted {
            id: input.id,
            name: input.name.clone(),
            corpus_snapshot: input.corpus_snapshot,
            fingerprint: input.fingerprint.clone(),
        };

        self.apply_index_generation_started(
            input.id,
            input.name,
            input.corpus_snapshot,
            input.fingerprint,
        )?;
        let envelope = self.emit_event(event);

        Ok(KernelOutput {
            events: vec![envelope.clone()],
            effects: vec![MaestriaEffect::PersistEvent {
                envelope: Box::new(envelope),
            }],
        })
    }

    pub(crate) fn apply_index_generation_started(
        &mut self,
        id: IndexGenerationId,
        name: crate::generations::RepresentationName,
        corpus_snapshot: crate::ids::CorpusSnapshotId,
        fingerprint: crate::generations::IndexFingerprint,
    ) -> Result<(), DomainError> {
        let generation = crate::generations::IndexGeneration {
            id,
            name,
            corpus_snapshot,
            fingerprint,
            lifecycle: crate::generations::IndexLifecycle::Building,
        };
        self.index_generations.register(generation)
    }

    pub(crate) fn process_transition_index_generation(
        &mut self,
        input: TransitionIndexGenerationInput,
    ) -> Result<KernelOutput, DomainError> {
        let generation = self
            .index_generations
            .get(input.id)
            .ok_or(DomainError::MissingIndexGeneration { id: input.id })?
            .clone();
        let replaced_active_id = if input.to == crate::generations::IndexLifecycle::Active {
            self.index_generations
                .active_id(&generation.name)
                .filter(|active_id| *active_id != input.id)
        } else {
            None
        };
        let event = DomainEvent::IndexGenerationTransitioned {
            id: input.id,
            from: generation.lifecycle,
            to: input.to,
            replaced_active_id,
        };

        self.apply_index_generation_transitioned(
            input.id,
            generation.lifecycle,
            input.to,
            replaced_active_id,
        )?;
        let envelope = self.emit_event(event);

        Ok(KernelOutput {
            events: vec![envelope.clone()],
            effects: vec![MaestriaEffect::PersistEvent {
                envelope: Box::new(envelope),
            }],
        })
    }

    pub(crate) fn apply_index_generation_transitioned(
        &mut self,
        id: IndexGenerationId,
        from: crate::generations::IndexLifecycle,
        to: crate::generations::IndexLifecycle,
        replaced_active_id: Option<IndexGenerationId>,
    ) -> Result<(), DomainError> {
        let generation = self
            .index_generations
            .get(id)
            .ok_or(DomainError::MissingIndexGeneration { id })?;
        if generation.lifecycle != from {
            return Err(DomainError::InvalidGenerationTransition {
                id,
                from: generation.lifecycle,
                to,
            });
        }
        let expected_replaced_active_id = if to == crate::generations::IndexLifecycle::Active {
            self.index_generations
                .active_id(&generation.name)
                .filter(|active_id| *active_id != id)
        } else {
            None
        };
        if expected_replaced_active_id != replaced_active_id {
            return Err(DomainError::InternalInvariantViolation {
                detail: "generation activation replacement mismatch",
            });
        }
        self.index_generations.transition_lifecycle(id, to)?;
        Ok(())
    }
}
