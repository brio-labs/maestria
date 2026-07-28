use super::{
    RecoveryQueueStage, queue_recovery_inputs, recovery_artifact_ids, validation_task_ids,
};
use crate::RecoveryInputs;
use maestria_domain::{
    ArtifactId, DomainInput, ParserStarted, RequestTaskValidation, StartFullTextIndex, TaskId,
};
use tokio::sync::mpsc;

fn parser_input(id: u64) -> DomainInput {
    DomainInput::ResumeParser(ParserStarted {
        artifact_id: ArtifactId::new(id),
        title: format!("artifact-{id}"),
        source_path: format!("/tmp/artifact-{id}"),
        content_hash: format!("hash-{id}"),
        blob_id: maestria_domain::BlobId::new(id),
    })
}

#[tokio::test]
async fn interrupted_queue_preserves_remaining_inputs_for_ordered_retry()
-> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(1);
    let mut recovery = RecoveryInputs {
        resume_parsers: vec![parser_input(1), parser_input(2)],
        start_full_text: vec![DomainInput::StartFullTextIndex(StartFullTextIndex {
            artifact_id: ArtifactId::new(3),
        })],
        run_validations: vec![DomainInput::RequestTaskValidation(RequestTaskValidation {
            task_id: TaskId::new(4),
        })],
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        queue_recovery_inputs(
            &input_tx,
            &mut recovery.resume_parsers,
            RecoveryQueueStage::ResumeParser,
        ),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(recovery.resume_parsers, vec![parser_input(2)]);
    assert_eq!(input_rx.recv().await, Some(parser_input(1)));

    let receiver = tokio::spawn(async move {
        let mut received = Vec::new();
        while let Some(input) = input_rx.recv().await {
            received.push(input);
        }
        received
    });
    queue_recovery_inputs(
        &input_tx,
        &mut recovery.resume_parsers,
        RecoveryQueueStage::ResumeParser,
    )
    .await?;
    queue_recovery_inputs(
        &input_tx,
        &mut recovery.start_full_text,
        RecoveryQueueStage::FullText,
    )
    .await?;
    queue_recovery_inputs(
        &input_tx,
        &mut recovery.run_validations,
        RecoveryQueueStage::Validation,
    )
    .await?;
    drop(input_tx);

    let mut received = vec![parser_input(1)];
    received.extend(receiver.await?);
    assert_eq!(
        received,
        vec![
            parser_input(1),
            parser_input(2),
            DomainInput::StartFullTextIndex(StartFullTextIndex {
                artifact_id: ArtifactId::new(3),
            }),
            DomainInput::RequestTaskValidation(RequestTaskValidation {
                task_id: TaskId::new(4),
            }),
        ]
    );
    assert!(recovery.resume_parsers.is_empty());
    assert!(recovery.start_full_text.is_empty());
    assert!(recovery.run_validations.is_empty());
    Ok(())
}

#[tokio::test]
async fn closed_receiver_preserves_unsent_inputs_with_stage_context()
-> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, input_rx) = mpsc::channel(1);
    drop(input_rx);
    let mut inputs = vec![DomainInput::RequestTaskValidation(RequestTaskValidation {
        task_id: TaskId::new(9),
    })];

    let error = queue_recovery_inputs(&input_tx, &mut inputs, RecoveryQueueStage::Validation)
        .await
        .err();
    let message = error.map_or_else(String::new, |error| format!("{error:#}"));
    assert!(message.contains("task validation"));
    assert_eq!(inputs.len(), 1);
    Ok(())
}

#[test]
fn recovery_queue_ids_preserve_dependency_groups() {
    let recovery = RecoveryInputs {
        resume_parsers: vec![parser_input(1)],
        start_full_text: vec![DomainInput::StartFullTextIndex(StartFullTextIndex {
            artifact_id: ArtifactId::new(2),
        })],
        run_validations: vec![DomainInput::RequestTaskValidation(RequestTaskValidation {
            task_id: TaskId::new(3),
        })],
    };
    assert_eq!(
        recovery_artifact_ids(&recovery),
        vec![ArtifactId::new(1), ArtifactId::new(2)]
    );
    assert_eq!(validation_task_ids(&recovery), vec![TaskId::new(3)]);
}
