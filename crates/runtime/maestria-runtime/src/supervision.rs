use crate::MaestriaRuntime;
use maestria_domain::{HarnessRunCompleted, HarnessRunId, MaestriaEffect};

impl MaestriaRuntime {
    pub(super) fn check_harness_feedback_boundary(&self, completion: &HarnessRunCompleted) -> bool {
        match self
            .adapters
            .effect_journal
            .is_feedback_accepted(completion.run_id, completion.generation)
        {
            Ok(true) => true,
            Ok(false) => {
                tracing::warn!(
                    run_id = %completion.run_id,
                    generation = %completion.generation,
                    "harness feedback rejected at runtime boundary"
                );
                false
            }
            Err(error) => {
                tracing::error!(%error, "failed to validate harness feedback generation");
                false
            }
        }
    }

    pub(super) fn register_harness_feedback(
        &self,
        feedback: Option<(HarnessRunId, u64)>,
        effects: &[MaestriaEffect],
    ) {
        let Some(feedback) = feedback else {
            return;
        };
        // The final persistence effect closes the whole domain outcome batch
        // (HarnessRunCompleted plus any task status transition).
        let event_id = effects.iter().rev().find_map(|effect| match effect {
            MaestriaEffect::PersistEvent { envelope } => Some(envelope.id),
            _ => None,
        });
        let Some(event_id) = event_id else {
            tracing::error!("harness completion produced no persistence effect");
            return;
        };
        match self.feedback_acks.lock() {
            Ok(mut pending) => {
                pending.insert(event_id, feedback);
            }
            Err(_) => tracing::error!("harness feedback acknowledgement lock poisoned"),
        }
    }
}
