use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use maestria_domain::{
    DomainInput, OcrCompleted, OcrCompletion, OcrEffect, OcrFailed, OcrPageText, content_hash,
};
use maestria_ports::{FileHandle, OcrRequest, OcrResponse};

impl EffectExecutionContext {
    pub(crate) async fn handle_ocr(&self, effect: OcrEffect) -> Result<(), EffectFailure> {
        let intent = effect.intent;
        let Some(provider) = &self.adapters.ocr_provider else {
            return self
                .ocr_failed(&intent, "OCR provider is not configured")
                .await;
        };
        if !Self::provider_contract_matches(&intent, provider.as_ref()) {
            return self
                .ocr_failed(
                    &intent,
                    "OCR provider contract changed after intent persistence",
                )
                .await;
        }
        let bytes = match self.adapters.blob_store.get(intent.source_blob()) {
            Ok(bytes) => bytes,
            Err(error) => {
                return self
                    .ocr_failed(&intent, &format!("load OCR source blob: {error}"))
                    .await;
            }
        };
        if content_hash(&bytes) != intent.source_hash().as_str() {
            return self
                .ocr_failed(
                    &intent,
                    "OCR source blob hash does not match persisted intent",
                )
                .await;
        }
        let request = OcrRequest {
            file: FileHandle {
                path: std::path::PathBuf::from(format!(
                    "artifact-{}.pdf",
                    intent.artifact_id().value()
                )),
                bytes,
            },
            pages: intent.pages().to_vec(),
        };
        let response = match provider.recognize(request) {
            Ok(response) => response,
            Err(error) => {
                return self
                    .ocr_failed(&intent, &format!("OCR provider failed: {error}"))
                    .await;
            }
        };
        if !self.response_contract_matches(&intent, &response) {
            return self
                .ocr_failed(
                    &intent,
                    "OCR provider response identity/disclosure mismatch",
                )
                .await;
        }
        let pages = match response
            .pages
            .into_iter()
            .map(|page| OcrPageText::new(page.page, page.text))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(pages) => pages,
            Err(error) => {
                return self
                    .ocr_failed(&intent, &format!("malformed OCR page result: {error}"))
                    .await;
            }
        };
        let completion = match OcrCompletion::new(&intent, pages) {
            Ok(completion) => completion,
            Err(error) => {
                return self
                    .ocr_failed(&intent, &format!("malformed OCR result: {error}"))
                    .await;
            }
        };
        Self::send_input(
            &self.input_tx,
            DomainInput::OcrCompleted(OcrCompleted {
                artifact_id: intent.artifact_id(),
                completion,
            }),
            "OCR completion",
        )
        .map_err(|error| EffectFailure::Failed(format!("send OCR completion: {error}")))
    }

    fn provider_contract_matches(
        intent: &maestria_domain::OcrIntent,
        provider: &dyn maestria_ports::OcrProvider,
    ) -> bool {
        let identity = provider.identity();
        let disclosure = provider.disclosure();
        let identity_matches = identity.provider == intent.provider().provider()
            && identity.model == intent.provider().model()
            && identity.revision == intent.provider().revision()
            && identity.artifact_hash == intent.provider().artifact_hash()
            && identity.preprocessing_version == intent.provider().preprocessing_version();
        let disclosure_matches = disclosure.remote == intent.disclosure().remote()
            && ((matches!(
                disclosure.retention,
                maestria_ports::RetentionPolicy::NoRetention
            ) && matches!(
                intent.disclosure().retention(),
                maestria_domain::OcrRetentionPolicy::NoRetention
            )) || (matches!(
                disclosure.retention,
                maestria_ports::RetentionPolicy::ProviderDefined
            ) && matches!(
                intent.disclosure().retention(),
                maestria_domain::OcrRetentionPolicy::ProviderDefined
            )));
        identity_matches && disclosure_matches
    }

    fn response_contract_matches(
        &self,
        intent: &maestria_domain::OcrIntent,
        response: &OcrResponse,
    ) -> bool {
        response.identity.provider == intent.provider().provider()
            && response.identity.model == intent.provider().model()
            && response.identity.revision == intent.provider().revision()
            && response.identity.artifact_hash == intent.provider().artifact_hash()
            && response.identity.preprocessing_version == intent.provider().preprocessing_version()
            && response.disclosure.remote == intent.disclosure().remote()
            && ((matches!(
                response.disclosure.retention,
                maestria_ports::RetentionPolicy::NoRetention
            ) && matches!(
                intent.disclosure().retention(),
                maestria_domain::OcrRetentionPolicy::NoRetention
            )) || (matches!(
                response.disclosure.retention,
                maestria_ports::RetentionPolicy::ProviderDefined
            ) && matches!(
                intent.disclosure().retention(),
                maestria_domain::OcrRetentionPolicy::ProviderDefined
            )))
    }

    async fn ocr_failed(
        &self,
        intent: &maestria_domain::OcrIntent,
        reason: &str,
    ) -> Result<(), EffectFailure> {
        if let Err(error) = Self::send_input(
            &self.input_tx,
            DomainInput::OcrFailed(OcrFailed {
                artifact_id: intent.artifact_id(),
                request_id: intent.request_id().clone(),
                reason: reason.to_string(),
            }),
            "OCR failure",
        ) {
            return Err(EffectFailure::Failed(format!("send OCR failure: {error}")));
        }
        Err(EffectFailure::Failed(reason.to_string()))
    }
}
