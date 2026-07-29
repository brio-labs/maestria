use super::event_payloads::StoredEventPayload;
use maestria_domain::{
    DomainEvent, OcrCompletion, OcrDisclosure, OcrIntent, OcrPageText, OcrProviderIdentity,
    OcrRequestId, OcrRetentionPolicy,
};
use maestria_ports::PortError;

impl StoredEventPayload {
    pub(crate) fn try_from_domain_ocr(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::OcrRequested { intent } => Some(Self::OcrRequested {
                request_id: intent.request_id().as_str().to_string(),
                artifact_id: intent.artifact_id().value(),
                source_blob: intent.source_blob().value(),
                source_hash: intent.source_hash().clone(),
                pages: intent.pages().to_vec(),
                provider: intent.provider().provider().to_string(),
                model: intent.provider().model().to_string(),
                revision: intent.provider().revision().to_string(),
                provider_artifact_hash: intent.provider().artifact_hash().to_string(),
                preprocessing_version: intent.provider().preprocessing_version().to_string(),
                remote: intent.disclosure().remote(),
                retention: match intent.disclosure().retention() {
                    OcrRetentionPolicy::NoRetention => "no_retention".to_string(),
                    OcrRetentionPolicy::ProviderDefined => "provider_defined".to_string(),
                },
            }),
            DomainEvent::OcrCompleted {
                artifact_id,
                completion,
            } => Some(Self::OcrCompleted {
                artifact_id: artifact_id.value(),
                request_id: completion.request_id().as_str().to_string(),
                pages: completion
                    .pages()
                    .iter()
                    .map(StoredOcrPage::from_domain)
                    .collect(),
            }),
            DomainEvent::OcrFailed {
                artifact_id,
                request_id,
                reason,
            } => Some(Self::OcrFailed {
                artifact_id: artifact_id.value(),
                request_id: request_id.as_str().to_string(),
                reason: reason.clone(),
            }),
            _ => None,
        }
    }

    pub(crate) fn try_into_domain_ocr(self) -> Result<DomainEvent, Box<Self>> {
        match self {
            payload @ Self::OcrRequested { .. } => Self::try_into_domain_ocr_requested(payload),
            payload @ Self::OcrCompleted { .. } => Self::try_into_domain_ocr_completed(payload),
            payload @ Self::OcrFailed { .. } => Self::try_into_domain_ocr_failed(payload),
            other => Err(Box::new(other)),
        }
    }

    fn try_into_domain_ocr_requested(self) -> Result<DomainEvent, Box<Self>> {
        let Self::OcrRequested {
            request_id,
            artifact_id,
            source_blob,
            source_hash,
            pages,
            provider,
            model,
            revision,
            provider_artifact_hash,
            preprocessing_version,
            remote,
            retention,
        } = self
        else {
            return Err(Box::new(self));
        };
        let provider = match OcrProviderIdentity::new(
            provider,
            model,
            revision,
            provider_artifact_hash,
            preprocessing_version,
        ) {
            Ok(value) => value,
            Err(_) => {
                return Err(Box::new(Self::OcrRequested {
                    request_id,
                    artifact_id,
                    source_blob,
                    source_hash,
                    pages,
                    provider: String::new(),
                    model: String::new(),
                    revision: String::new(),
                    provider_artifact_hash: String::new(),
                    preprocessing_version: String::new(),
                    remote,
                    retention,
                }));
            }
        };
        let retention = match Self::decode_ocr_retention(&retention) {
            Some(value) => value,
            None => {
                return Err(Box::new(Self::OcrRequested {
                    request_id,
                    artifact_id,
                    source_blob,
                    source_hash,
                    pages,
                    provider: provider.provider().to_string(),
                    model: provider.model().to_string(),
                    revision: provider.revision().to_string(),
                    provider_artifact_hash: provider.artifact_hash().to_string(),
                    preprocessing_version: provider.preprocessing_version().to_string(),
                    remote,
                    retention,
                }));
            }
        };
        let intent = OcrIntent::new(
            maestria_domain::ArtifactId::new(artifact_id),
            maestria_domain::BlobId::new(source_blob),
            source_hash,
            pages,
            provider,
            OcrDisclosure::new(remote, retention),
        )
        .map_err(|_| {
            Box::new(Self::OcrFailed {
                artifact_id,
                request_id: request_id.clone(),
                reason: "invalid OCR intent payload".to_string(),
            })
        })?;
        if intent.request_id().as_str() != request_id {
            return Err(Self::invalid_ocr_request_id(
                request_id,
                artifact_id,
                source_blob,
                remote,
                &intent,
            ));
        }
        Ok(DomainEvent::OcrRequested { intent })
    }
    fn try_into_domain_ocr_completed(self) -> Result<DomainEvent, Box<Self>> {
        let Self::OcrCompleted {
            artifact_id,
            request_id,
            pages,
        } = self
        else {
            return Err(Box::new(self));
        };
        let request_id = match OcrRequestId::parse(request_id.clone()) {
            Ok(value) => value,
            Err(_) => {
                return Err(Box::new(Self::OcrCompleted {
                    artifact_id,
                    request_id,
                    pages,
                }));
            }
        };
        let decoded = pages
            .into_iter()
            .map(StoredOcrPage::into_domain)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| {
                Box::new(Self::OcrCompleted {
                    artifact_id,
                    request_id: request_id.as_str().to_string(),
                    pages: Vec::new(),
                })
            })?;
        let completion = OcrCompletion::from_parts(request_id, decoded).map_err(|_| {
            Box::new(Self::OcrCompleted {
                artifact_id,
                request_id: String::new(),
                pages: Vec::new(),
            })
        })?;
        Ok(DomainEvent::OcrCompleted {
            artifact_id: maestria_domain::ArtifactId::new(artifact_id),
            completion,
        })
    }

    fn try_into_domain_ocr_failed(self) -> Result<DomainEvent, Box<Self>> {
        let Self::OcrFailed {
            artifact_id,
            request_id,
            reason,
        } = self
        else {
            return Err(Box::new(self));
        };
        let request_id = OcrRequestId::parse(request_id).map_err(|_| {
            Box::new(Self::OcrFailed {
                artifact_id,
                request_id: String::new(),
                reason: reason.clone(),
            })
        })?;
        Ok(DomainEvent::OcrFailed {
            artifact_id: maestria_domain::ArtifactId::new(artifact_id),
            request_id,
            reason,
        })
    }

    fn decode_ocr_retention(value: &str) -> Option<OcrRetentionPolicy> {
        match value {
            "no_retention" => Some(OcrRetentionPolicy::NoRetention),
            "provider_defined" => Some(OcrRetentionPolicy::ProviderDefined),
            _ => None,
        }
    }

    fn invalid_ocr_request_id(
        request_id: String,
        artifact_id: u64,
        source_blob: u64,
        remote: bool,
        intent: &OcrIntent,
    ) -> Box<Self> {
        Box::new(Self::OcrRequested {
            request_id,
            artifact_id,
            source_blob,
            source_hash: intent.source_hash().clone(),
            pages: intent.pages().to_vec(),
            provider: intent.provider().provider().to_string(),
            model: intent.provider().model().to_string(),
            revision: intent.provider().revision().to_string(),
            provider_artifact_hash: intent.provider().artifact_hash().to_string(),
            preprocessing_version: intent.provider().preprocessing_version().to_string(),
            remote,
            retention: match intent.disclosure().retention() {
                OcrRetentionPolicy::NoRetention => "no_retention".to_string(),
                OcrRetentionPolicy::ProviderDefined => "provider_defined".to_string(),
            },
        })
    }

    pub(crate) fn try_kind_ocr(&self) -> Option<&'static str> {
        match self {
            Self::OcrRequested { .. } => Some("ocr_requested"),
            Self::OcrCompleted { .. } => Some("ocr_completed"),
            Self::OcrFailed { .. } => Some("ocr_failed"),
            _ => None,
        }
    }
    pub(crate) fn try_filter_artifact_id_ocr(&self) -> Option<u64> {
        match self {
            Self::OcrRequested { artifact_id, .. }
            | Self::OcrCompleted { artifact_id, .. }
            | Self::OcrFailed { artifact_id, .. } => Some(*artifact_id),
            _ => None,
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct StoredOcrPage {
    page: u32,
    text: String,
}
impl StoredOcrPage {
    fn from_domain(page: &OcrPageText) -> Self {
        Self {
            page: page.page(),
            text: page.text().to_string(),
        }
    }
    fn into_domain(self) -> Result<OcrPageText, PortError> {
        OcrPageText::new(self.page, self.text).map_err(|error| PortError::InvalidInputContext {
            context: "decode OCR page",
            source: error.to_string(),
        })
    }
}
