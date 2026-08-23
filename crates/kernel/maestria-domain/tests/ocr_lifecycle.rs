use maestria_domain::*;
use std::sync::Arc;

fn intent() -> Result<OcrIntent, Box<dyn std::error::Error>> {
    Ok(OcrIntent::new(
        ArtifactId::new(7),
        BlobId::new(11),
        ContentHash::new(format!("sha256:{}", "0".repeat(64)))?,
        [2, 1],
        OcrProviderIdentity::new("fixture", "ocr", "v1", "sha256:provider", "prep-v1")?,
        OcrDisclosure::new(false, OcrRetentionPolicy::NoRetention),
    )?)
}

#[test]
fn intent_is_durable_before_ocr_effect_and_pages_are_exact()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    Arc::make_mut(&mut state.pending_parsers).insert(
        ArtifactId::new(7),
        ParserStarted {
            artifact_id: ArtifactId::new(7),
            title: "scan".into(),
            source_path: "scan.pdf".into(),
            content_hash: ContentHash::new(format!("sha256:{:064x}", 4))?,
            blob_id: BlobId::new(11),
        },
    );
    let request = intent()?;
    let output = state.apply_input(DomainInput::OcrRequested(OcrRequested {
        intent: request.clone(),
    }))?;
    assert!(matches!(
        output.events[0].event,
        DomainEvent::OcrRequested { .. }
    ));
    assert!(matches!(
        output.effects[0],
        MaestriaEffect::PersistEvent { .. }
    ));
    assert!(matches!(output.effects[1], MaestriaEffect::Ocr(_)));
    assert!(state.pending_ocr.contains_key(request.request_id()));
    assert!(
        OcrCompletion::new(
            &request,
            [OcrPageText::new(1, "one")?, OcrPageText::new(2, "two")?]
        )
        .is_ok()
    );
    Ok(())
}

#[test]
fn malformed_ocr_pages_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let request = intent()?;
    assert!(OcrPageText::new(0, "zero").is_err());
    assert!(
        OcrCompletion::new(
            &request,
            [
                OcrPageText::new(1, "one")?,
                OcrPageText::new(1, "duplicate")?,
            ]
        )
        .is_err()
    );
    assert!(
        OcrCompletion::new(
            &request,
            [OcrPageText::new(1, "one")?, OcrPageText::new(2, "two")?,]
        )
        .is_ok()
    );
    Ok(())
}

#[test]
fn durable_completion_replay_keeps_result_without_pending_transport_intent()
-> Result<(), Box<dyn std::error::Error>> {
    let request = intent()?;
    let completion = OcrCompletion::new(
        &request,
        [OcrPageText::new(1, "one")?, OcrPageText::new(2, "two")?],
    )?;
    let events = vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            event: DomainEvent::OcrRequested {
                intent: request.clone(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            event: DomainEvent::OcrCompleted {
                artifact_id: request.artifact_id(),
                completion: completion.clone(),
            },
        },
    ];
    let state = replay_events(events)?;
    assert!(state.pending_ocr.is_empty());
    assert_eq!(
        state.ocr_results.get(request.request_id()),
        Some(&completion)
    );
    Ok(())
}

#[test]
fn replay_rejects_uncorrelated_or_conflicting_ocr_terminals()
-> Result<(), Box<dyn std::error::Error>> {
    let request = intent()?;
    let completion = OcrCompletion::new(
        &request,
        [OcrPageText::new(1, "one")?, OcrPageText::new(2, "two")?],
    )?;
    let requested = DomainEventEnvelope {
        id: EventId::new(1),
        event: DomainEvent::OcrRequested {
            intent: request.clone(),
        },
    };
    let completed = DomainEventEnvelope {
        id: EventId::new(2),
        event: DomainEvent::OcrCompleted {
            artifact_id: request.artifact_id(),
            completion: completion.clone(),
        },
    };
    let duplicate = DomainEventEnvelope {
        id: EventId::new(3),
        event: DomainEvent::OcrCompleted {
            artifact_id: request.artifact_id(),
            completion: completion.clone(),
        },
    };
    let replayed = replay_events([requested.clone(), completed.clone(), duplicate].to_vec())?;
    assert!(replayed.pending_ocr.is_empty());
    assert_eq!(
        replayed.ocr_results.get(request.request_id()),
        Some(&completion)
    );

    let conflicting_completion = OcrCompletion::from_parts(
        completion.request_id().clone(),
        vec![OcrPageText::new(1, "changed")?, OcrPageText::new(2, "two")?],
    )?;
    let conflict = DomainEventEnvelope {
        id: EventId::new(3),
        event: DomainEvent::OcrCompleted {
            artifact_id: request.artifact_id(),
            completion: conflicting_completion,
        },
    };
    assert!(replay_events([requested.clone(), completed.clone(), conflict].to_vec()).is_err());

    let wrong_artifact = DomainEventEnvelope {
        id: EventId::new(2),
        event: DomainEvent::OcrCompleted {
            artifact_id: ArtifactId::new(8),
            completion,
        },
    };
    assert!(replay_events([requested.clone(), wrong_artifact].to_vec()).is_err());

    let uncorrelated_failure = DomainEventEnvelope {
        id: EventId::new(1),
        event: DomainEvent::OcrFailed {
            artifact_id: request.artifact_id(),
            request_id: request.request_id().clone(),
            reason: "failed".to_string(),
        },
    };
    assert!(replay_events([uncorrelated_failure].to_vec()).is_err());
    Ok(())
}

#[test]
fn ocr_failure_terminalizes_parser_in_live_state_and_replay()
-> Result<(), Box<dyn std::error::Error>> {
    let request = intent()?;
    let parser = ParserStarted {
        artifact_id: request.artifact_id(),
        title: "scan".to_string(),
        source_path: "scan.pdf".to_string(),
        content_hash: request.source_hash().clone(),
        blob_id: request.source_blob(),
    };
    let failure = OcrFailed {
        artifact_id: request.artifact_id(),
        request_id: request.request_id().clone(),
        reason: "provider failed".to_string(),
    };
    let mut state = KernelState::new();
    Arc::make_mut(&mut state.pending_parsers).insert(request.artifact_id(), parser.clone());
    state.apply_input(DomainInput::OcrRequested(OcrRequested {
        intent: request.clone(),
    }))?;
    state.apply_input(DomainInput::OcrFailed(failure.clone()))?;
    assert!(!state.pending_parsers.contains_key(&request.artifact_id()));
    assert!(!state.pending_ocr.contains_key(request.request_id()));

    let replayed = replay_events(
        [
            DomainEventEnvelope {
                id: EventId::new(1),
                event: DomainEvent::ParserStarted {
                    artifact_id: parser.artifact_id,
                    title: parser.title,
                    source_path: parser.source_path,
                    content_hash: parser.content_hash,
                    blob_id: parser.blob_id,
                },
            },
            DomainEventEnvelope {
                id: EventId::new(2),
                event: DomainEvent::OcrRequested { intent: request },
            },
            DomainEventEnvelope {
                id: EventId::new(3),
                event: DomainEvent::OcrFailed {
                    artifact_id: failure.artifact_id,
                    request_id: failure.request_id,
                    reason: failure.reason,
                },
            },
        ]
        .to_vec(),
    )?;
    assert!(replayed.pending_parsers.is_empty());
    assert!(replayed.pending_ocr.is_empty());
    Ok(())
}

#[test]
fn ocr_request_id_requires_sha256_hex_digest() -> Result<(), Box<dyn std::error::Error>> {
    let valid = format!("ocr:sha256:{}", "a".repeat(64));
    assert_eq!(OcrRequestId::parse(valid.clone())?.as_str(), valid);
    assert!(
        OcrRequestId::parse(
            "ocr:sha256:not-hex-0000000000000000000000000000000000000000000000000000000000"
        )
        .is_err()
    );
    assert!(OcrRequestId::parse(format!("ocr:sha256:{}", "a".repeat(63))).is_err());
    assert!(OcrRequestId::parse(format!("ocr:sha256:{}", "a".repeat(65))).is_err());
    assert!(
        OcrRequestId::parse(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .is_err()
    );
    Ok(())
}
