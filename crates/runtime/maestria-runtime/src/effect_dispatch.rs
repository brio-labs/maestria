use crate::MaestriaRuntime;
use crate::effect_execution_dispatch::PreparedEffect;
use maestria_domain::MaestriaEffect;
use std::fmt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(crate) enum EffectWork {
    Pending(MaestriaEffect),
    Prepared(PreparedEffect),
}

pub(crate) type EffectBatch = Vec<EffectWork>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectAdmissionError {
    Cancelled,
    ChannelClosed,
}

impl fmt::Display for EffectAdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("effect batch admission cancelled"),
            Self::ChannelClosed => f.write_str("effect channel closed during batch admission"),
        }
    }
}

impl std::error::Error for EffectAdmissionError {}

impl MaestriaRuntime {
    /// Reserve one channel item for the complete effect batch before the
    /// domain transition is committed. A successful reservation is the
    /// all-or-none admission cutoff.
    pub(crate) async fn reserve_effect_batch<'a>(
        &self,
        effect_tx: &'a mpsc::Sender<EffectBatch>,
        shutdown_token: &CancellationToken,
    ) -> Result<mpsc::Permit<'a, EffectBatch>, EffectAdmissionError> {
        tokio::select! {
            biased;
            () = shutdown_token.cancelled() => Err(EffectAdmissionError::Cancelled),
            result = effect_tx.reserve() => {
                result.map_err(|_| EffectAdmissionError::ChannelClosed)
            }
        }
    }

    /// Send an already-reserved batch. The vector is one bounded channel
    /// item, so no prefix can be admitted independently of the transition.
    pub(crate) fn send_reserved_effects(
        &self,
        permit: mpsc::Permit<'_, EffectBatch>,
        effects: EffectBatch,
    ) -> Result<(), EffectAdmissionError> {
        permit.send(effects);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::UpdateGraphRequest;

    #[tokio::test]
    async fn admission_carries_a_large_transition_batch_as_one_item()
    -> Result<(), Box<dyn std::error::Error>> {
        let (sender, mut receiver) = mpsc::channel::<EffectBatch>(1);
        let batch = (0..8)
            .map(|index| {
                EffectWork::Pending(MaestriaEffect::UpdateGraph(UpdateGraphRequest {
                    relation_id: maestria_domain::RelationId::new(index as u64),
                }))
            })
            .collect::<Vec<_>>();
        let expected_len = batch.len();
        let permit = sender.reserve().await?;
        permit.send(batch);

        let admitted = receiver.recv().await.ok_or("batch should be admitted")?;
        assert_eq!(admitted.len(), expected_len);
        Ok(())
    }
}
