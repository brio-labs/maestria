use super::event_payloads::{FamilyDecodeError, StoredEventPayload};
use super::stored_content::StoredContentHash;
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
                source_hash: StoredContentHash::from_domain(intent.source_hash()),
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

    pub(crate) fn try_into_domain_ocr(self) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
            payload @ Self::OcrRequested { .. } => Self::try_into_domain_ocr_requested(payload),
            payload @ Self::OcrCompleted { .. } => Self::try_into_domain_ocr_completed(payload),
            payload @ Self::OcrFailed { .. } => Self::try_into_domain_ocr_failed(payload),
            other => Err(FamilyDecodeError::Foreign(Box::new(other))),
        }
    }

    fn try_into_domain_ocr_requested(self) -> Result<DomainEvent, FamilyDecodeError> {
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
            return Err(FamilyDecodeError::Foreign(Box::new(self)));
        };
        let source_hash = source_hash
            .try_into_domain()
            .map_err(FamilyDecodeError::Invalid)?;
        let provider = match OcrProviderIdentity::new(
            provider,
            model,
            revision,
            provider_artifact_hash,
            preprocessing_version,
        ) {
            Ok(value) => value,
            Err(error) => {
                return Err(FamilyDecodeError::Invalid(PortError::InvalidInputContext {
                    context: "decode stored OCR provider identity",
                    source: error.to_string(),
                }));
            }
        };
        let retention = match Self::decode_ocr_retention(&retention) {
            Some(value) => value,
            None => {
                return Err(FamilyDecodeError::Invalid(PortError::InvalidInputContext {
                    context: "decode stored OCR retention policy",
                    source: retention.clone(),
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
        .map_err(|error| {
            FamilyDecodeError::Invalid(PortError::InvalidInputContext {
                context: "decode stored OCR intent",
                source: error.to_string(),
            })
        })?;
        if intent.request_id().as_str() != request_id {
            return Err(FamilyDecodeError::Invalid(PortError::InvalidInputContext {
                context: "decode stored OCR request",
                source: "request id does not match the reconstructed intent".to_string(),
            }));
        }
        Ok(DomainEvent::OcrRequested { intent })
    }
    fn try_into_domain_ocr_completed(self) -> Result<DomainEvent, FamilyDecodeError> {
        let Self::OcrCompleted {
            artifact_id,
            request_id,
            pages,
        } = self
        else {
            return Err(FamilyDecodeError::Foreign(Box::new(self)));
        };
        let request_id = match OcrRequestId::parse(request_id.clone()) {
            Ok(value) => value,
            Err(error) => {
                return Err(FamilyDecodeError::Invalid(PortError::InvalidInputContext {
                    context: "decode stored OCR request id",
                    source: error.to_string(),
                }));
            }
        };
        let decoded = pages
            .into_iter()
            .map(StoredOcrPage::into_domain)
            .collect::<Result<Vec<_>, _>>()
            .map_err(FamilyDecodeError::Invalid)?;
        let completion = OcrCompletion::from_parts(request_id, decoded).map_err(|error| {
            FamilyDecodeError::Invalid(PortError::InvalidInputContext {
                context: "decode stored OCR completion",
                source: error.to_string(),
            })
        })?;
        Ok(DomainEvent::OcrCompleted {
            artifact_id: maestria_domain::ArtifactId::new(artifact_id),
            completion,
        })
    }

    fn try_into_domain_ocr_failed(self) -> Result<DomainEvent, FamilyDecodeError> {
        let Self::OcrFailed {
            artifact_id,
            request_id,
            reason,
        } = self
        else {
            return Err(FamilyDecodeError::Foreign(Box::new(self)));
        };
        let request_id = OcrRequestId::parse(request_id).map_err(|error| {
            FamilyDecodeError::Invalid(PortError::InvalidInputContext {
                context: "decode stored OCR failure request id",
                source: error.to_string(),
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
