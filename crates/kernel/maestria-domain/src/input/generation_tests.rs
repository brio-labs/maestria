use crate::generations::{IndexFingerprint, IndexGeneration, IndexLifecycle, RepresentationName};
use crate::replay_events;
use crate::search::ContentHash;
use crate::types::*;

fn fingerprint() -> Result<IndexFingerprint, DomainError> {
    Ok(IndexFingerprint {
        provider: "test-provider".into(),
        model: "test-model".into(),
        revision: "v1".into(),
        artifact_hash: ContentHash::try_from(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        )
        .map_err(|_| DomainError::InternalInvariantViolation {
            detail: "generation test fingerprint must be valid",
        })?,
        dimensions: 1536,
        quantization: "fp32".into(),
        query_template_hash: "qt-hash".into(),
        document_template_hash: "dt-hash".into(),
        preprocessing_version: "1.0".into(),
    })
}

fn start(state: &mut KernelState, id: u64, name: &RepresentationName) -> Result<(), DomainError> {
    let output = state.process_start_index_generation(StartIndexGenerationInput {
        id: IndexGenerationId::new(id),
        name: name.clone(),
        corpus_snapshot: CorpusSnapshotId::new(1),
        fingerprint: fingerprint()?,
    })?;
    assert!(matches!(
        output.effects.as_slice(),
        [MaestriaEffect::PersistEvent { .. }]
    ));
    Ok(())
}

fn activate(state: &mut KernelState, id: u64) -> Result<(), DomainError> {
    let id = IndexGenerationId::new(id);
    for lifecycle in [IndexLifecycle::Evaluated, IndexLifecycle::Shadow] {
        state.process_transition_index_generation(TransitionIndexGenerationInput {
            id,
            to: lifecycle,
        })?;
    }
    state
        .process_transition_index_generation(TransitionIndexGenerationInput {
            id,
            to: IndexLifecycle::Active,
        })
        .map(|_| ())
}

fn generation(state: &KernelState, id: u64) -> Result<&IndexGeneration, DomainError> {
    state
        .index_generations
        .get(IndexGenerationId::new(id))
        .ok_or(DomainError::MissingIndexGeneration {
            id: IndexGenerationId::new(id),
        })
}

#[test]
fn lifecycle_serves_only_active_and_tombstones_cannot_reactivate() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let name = RepresentationName::new("dense_text_v1");
    start(&mut state, 1, &name)?;
    assert!(!generation(&state, 1)?.is_serveable());

    activate(&mut state, 1)?;
    assert!(
        state
            .index_generations
            .get_active(&name)
            .ok_or(DomainError::MissingIndexGeneration {
                id: IndexGenerationId::new(1),
            })?
            .is_serveable()
    );

    for lifecycle in [
        IndexLifecycle::Retired,
        IndexLifecycle::Collectable,
        IndexLifecycle::Tombstoned,
    ] {
        state.process_transition_index_generation(TransitionIndexGenerationInput {
            id: IndexGenerationId::new(1),
            to: lifecycle,
        })?;
    }
    assert!(
        state
            .process_transition_index_generation(TransitionIndexGenerationInput {
                id: IndexGenerationId::new(1),
                to: IndexLifecycle::Active,
            })
            .is_err()
    );
    Ok(())
}

#[test]
fn activation_replacement_and_rollback_preserve_fingerprints() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let name = RepresentationName::new("lexical_body");
    start(&mut state, 1, &name)?;
    activate(&mut state, 1)?;
    let original = generation(&state, 1)?.fingerprint.clone();

    start(&mut state, 2, &name)?;
    let output_before = state.event_log.len();
    assert!(
        state
            .process_transition_index_generation(TransitionIndexGenerationInput {
                id: IndexGenerationId::new(2),
                to: IndexLifecycle::Active,
            })
            .is_err()
    );
    assert_eq!(state.event_log.len(), output_before);
    assert_eq!(generation(&state, 1)?.lifecycle, IndexLifecycle::Active);
    assert!(
        state
            .index_generations
            .is_serveable(IndexGenerationId::new(1))
    );
    assert_eq!(
        state.index_generations.get_active(&name).map(|g| g.id),
        Some(IndexGenerationId::new(1))
    );
    activate(&mut state, 2)?;
    assert_eq!(state.event_log.len(), output_before + 3);
    assert_eq!(
        state.index_generations.get_active(&name).map(|g| g.id),
        Some(IndexGenerationId::new(2))
    );
    assert_eq!(generation(&state, 1)?.lifecycle, IndexLifecycle::Retired);

    state.process_transition_index_generation(TransitionIndexGenerationInput {
        id: IndexGenerationId::new(1),
        to: IndexLifecycle::Active,
    })?;
    assert_eq!(
        state.index_generations.get_active(&name).map(|g| g.id),
        Some(IndexGenerationId::new(1))
    );
    assert_eq!(generation(&state, 1)?.fingerprint, original);
    Ok(())
}

#[test]
fn event_replay_reconstructs_generation_registry() -> Result<(), DomainError> {
    let mut original = KernelState::new();
    let name = RepresentationName::new("code_dense_v1");
    start(&mut original, 7, &name)?;
    activate(&mut original, 7)?;
    let replayed = replay_events(&original.event_log)?;
    assert_eq!(original.index_generations, replayed.index_generations);
    assert_eq!(
        replayed.index_generations.get_active(&name).map(|g| g.id),
        Some(IndexGenerationId::new(7))
    );
    Ok(())
}
