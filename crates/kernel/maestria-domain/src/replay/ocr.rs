use crate::types::*;
use crate::{OcrCompletion, OcrIntent, OcrRequestId};
use std::sync::Arc;

impl KernelState {
    pub(super) fn replay_ocr_requested(&mut self, intent: &OcrIntent) -> Result<(), DomainError> {
        let request_id = intent.request_id().clone();
        if let Some(existing) = self.ocr_intents.get(&request_id) {
            if existing != intent {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR request replay conflicts with an existing intent",
                });
            }
            if self.ocr_results.contains_key(&request_id)
                || self.ocr_failures.contains_key(&request_id)
            {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR request replay would resurrect a terminal OCR result",
                });
            }
            if let Some(pending) = self.pending_ocr.get(&request_id) {
                if pending == intent {
                    return Ok(());
                }
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR request replay conflicts with a pending intent",
                });
            }
        }
        Arc::make_mut(&mut self.pending_ocr).insert(request_id.clone(), intent.clone());
        Arc::make_mut(&mut self.ocr_intents).insert(request_id, intent.clone());
        Ok(())
    }

    pub(super) fn replay_ocr_completed(
        &mut self,
        artifact_id: ArtifactId,
        completion: &OcrCompletion,
    ) -> Result<(), DomainError> {
        if let Some(existing) = self.ocr_results.get(completion.request_id()) {
            let Some(intent) = self.ocr_intents.get(completion.request_id()) else {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR completion replay has no request intent",
                });
            };
            if artifact_id != intent.artifact_id() {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR completion replay artifact does not correlate with intent",
                });
            }
            if existing == completion {
                return Ok(());
            }
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR completion replay conflicts with an existing terminal result",
            });
        }
        if self.ocr_failures.contains_key(completion.request_id()) {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR completion replay conflicts with an existing OCR failure",
            });
        }
        let Some(intent) = self.ocr_intents.get(completion.request_id()).cloned() else {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR completion replay has no request intent",
            });
        };
        if !self.pending_ocr.contains_key(completion.request_id()) {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR completion replay has no pending intent",
            });
        }
        self.validate_ocr_correlation(artifact_id, completion, &intent)?;
        Arc::make_mut(&mut self.pending_ocr).remove(completion.request_id());
        Arc::make_mut(&mut self.ocr_results)
            .insert(completion.request_id().clone(), completion.clone());
        Ok(())
    }

    pub(super) fn replay_ocr_failed(
        &mut self,
        artifact_id: ArtifactId,
        request_id: &OcrRequestId,
        reason: &str,
    ) -> Result<(), DomainError> {
        if let Some(existing) = self.ocr_failures.get(request_id) {
            let Some(intent) = self.ocr_intents.get(request_id) else {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR failure replay has no request intent",
                });
            };
            if artifact_id != intent.artifact_id() {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR failure replay artifact does not correlate with intent",
                });
            }
            if existing == reason {
                return Ok(());
            }
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR failure replay conflicts with an existing terminal result",
            });
        }
        if self.ocr_results.contains_key(request_id) {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR failure replay conflicts with an existing OCR completion",
            });
        }
        let Some(intent) = self.ocr_intents.get(request_id) else {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR failure replay has no request intent",
            });
        };
        if !self.pending_ocr.contains_key(request_id) {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR failure replay has no pending intent",
            });
        }
        if artifact_id != intent.artifact_id() {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR failure replay artifact does not correlate with intent",
            });
        }
        Arc::make_mut(&mut self.pending_parsers).remove(&artifact_id);
        Arc::make_mut(&mut self.pending_ocr).remove(request_id);
        Arc::make_mut(&mut self.ocr_failures).insert(request_id.clone(), reason.to_string());
        Ok(())
    }
}
