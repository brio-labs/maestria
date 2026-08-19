use crate::effects::{MaestriaEffect, ParseArtifactRequest, ParseArtifactSource};
use crate::events::DomainEvent;
use crate::inputs::{OcrCompleted, OcrFailed, OcrRequested};
use crate::ocr::{OcrCompletion, OcrIntent};
use crate::{DomainError, KernelOutput};

impl crate::KernelState {
    /// Shared OCR correlation validation used by the live handlers and the
    /// replay appliers: the completion must reference the intent's artifact
    /// and correlate exactly with the intent.
    pub(crate) fn validate_ocr_correlation(
        &self,
        artifact_id: crate::ids::ArtifactId,
        completion: &OcrCompletion,
        intent: &OcrIntent,
    ) -> Result<(), DomainError> {
        if artifact_id != intent.artifact_id() {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR completion artifact does not correlate with intent",
            });
        }
        completion
            .validate_against(intent)
            .map_err(|_| DomainError::InternalInvariantViolation {
                detail: "OCR completion does not correlate exactly with its intent",
            })
    }

    pub(super) fn process_ocr_requested(
        &mut self,
        input: OcrRequested,
    ) -> Result<KernelOutput, DomainError> {
        let intent = input.intent;
        let request_id = intent.request_id().clone();
        if let Some(existing) = self.pending_ocr.get(&request_id) {
            if existing == &intent {
                return Ok(KernelOutput {
                    effects: vec![MaestriaEffect::Ocr(intent)],
                    ..KernelOutput::default()
                });
            }
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR request identity is already bound to a different intent",
            });
        }
        if self.ocr_results.contains_key(&request_id) || self.ocr_failures.contains_key(&request_id)
        {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR request identity was already terminal",
            });
        }
        let event = self.emit_event(DomainEvent::OcrRequested {
            intent: intent.clone(),
        });
        let mut output = Self::output_for_event(event);
        self.pending_ocr.insert(request_id.clone(), intent.clone());
        self.ocr_intents.insert(request_id.clone(), intent.clone());
        output.effects.push(MaestriaEffect::Ocr(intent));
        Ok(output)
    }
    pub(super) fn process_ocr_completed(
        &mut self,
        input: OcrCompleted,
    ) -> Result<KernelOutput, DomainError> {
        let completion = input.completion;
        let request_id = completion.request_id().clone();
        if let Some(existing) = self.ocr_results.get(&request_id) {
            let Some(intent) = self.ocr_intents.get(&request_id) else {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR completion has no request intent",
                });
            };
            if input.artifact_id != intent.artifact_id() {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR completion artifact does not correlate with intent",
                });
            }
            if existing == &completion {
                return Ok(KernelOutput::default());
            }
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR completion conflicts with an existing terminal result",
            });
        }
        if self.ocr_failures.contains_key(&request_id) {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR completion conflicts with an existing OCR failure",
            });
        }
        let Some(intent) = self.pending_ocr.get(&request_id).cloned() else {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR completion has no pending intent",
            });
        };
        self.validate_ocr_correlation(input.artifact_id, &completion, &intent)?;
        let parser = self
            .pending_parsers
            .get(&intent.artifact_id())
            .cloned()
            .ok_or(DomainError::InternalInvariantViolation {
                detail: "OCR completion has no pending parser",
            })?;
        let event = self.emit_event(DomainEvent::OcrCompleted {
            artifact_id: input.artifact_id,
            completion: completion.clone(),
        });
        let mut output = Self::output_for_event(event);
        self.pending_ocr.remove(&request_id);
        self.ocr_results.insert(request_id, completion);
        output
            .effects
            .push(MaestriaEffect::ParseArtifact(ParseArtifactRequest {
                artifact_id: intent.artifact_id(),
                source_path: parser.source_path,
                source: ParseArtifactSource::Blob(intent.source_blob()),
            }));
        Ok(output)
    }

    pub(super) fn process_ocr_failed(
        &mut self,
        input: OcrFailed,
    ) -> Result<KernelOutput, DomainError> {
        if let Some(existing) = self.ocr_failures.get(&input.request_id) {
            let Some(intent) = self.ocr_intents.get(&input.request_id) else {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR failure has no request intent",
                });
            };
            if input.artifact_id != intent.artifact_id() {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "OCR failure artifact does not correlate with intent",
                });
            }
            if existing == &input.reason {
                return Ok(KernelOutput::default());
            }
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR failure conflicts with an existing terminal result",
            });
        }
        if self.ocr_results.contains_key(&input.request_id) {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR failure conflicts with an existing OCR completion",
            });
        }
        let Some(intent) = self.pending_ocr.get(&input.request_id) else {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR failure has no pending intent",
            });
        };
        if input.artifact_id != intent.artifact_id() {
            return Err(DomainError::InternalInvariantViolation {
                detail: "OCR failure artifact does not correlate with intent",
            });
        }
        let event = self.emit_event(DomainEvent::OcrFailed {
            artifact_id: input.artifact_id,
            request_id: input.request_id.clone(),
            reason: input.reason.clone(),
        });
        self.pending_ocr.remove(&input.request_id);
        self.pending_parsers.remove(&input.artifact_id);
        self.ocr_failures.insert(input.request_id, input.reason);
        Ok(Self::output_for_event(event))
    }
}
