use crate::MaestriaRuntime;
use maestria_domain::MaestriaEffect;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

impl MaestriaRuntime {
    pub(crate) async fn dispatch_effects(
        &self,
        effects: Vec<MaestriaEffect>,
        effect_tx: &mpsc::Sender<MaestriaEffect>,
        shutdown_token: &CancellationToken,
    ) -> bool {
        for effect in effects {
            if self.config.drain_effects_on_shutdown {
                if let Err(error) = effect_tx.send(effect).await {
                    tracing::error!(%error, "failed to dispatch effect");
                    shutdown_token.cancel();
                    return false;
                }
            } else {
                tokio::select! {
                    () = shutdown_token.cancelled() => return false,
                    result = effect_tx.send(effect) => {
                        if let Err(error) = result {
                            tracing::error!(%error, "failed to dispatch effect");
                            shutdown_token.cancel();
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}
