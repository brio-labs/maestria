use maestria_domain::*;

fn sample_inputs() -> Vec<DomainInput> {
    vec![
        DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: ArtifactId::new(1),
            title: "Project Notes".to_string(),
            source_path: "notes.txt".to_string(),
            source_bytes: b"project notes content".to_vec(),
            content_hash: "sha256:abc".to_string(),
        }),
        DomainInput::ParserCompleted(ParserResult {
            artifact_id: ArtifactId::new(1),
            chunks: vec![
                RegisterChunkInput {
                    chunk_id: ChunkId::new(10),
                    artifact_id: ArtifactId::new(1),
                    order: 0,
                    text: "first chunk".to_string(),
                },
                RegisterChunkInput {
                    chunk_id: ChunkId::new(11),
                    artifact_id: ArtifactId::new(1),
                    order: 1,
                    text: "second chunk".to_string(),
                },
            ],
            cards: Vec::new(),
        }),
        DomainInput::CreateClaim(CreateClaimInput {
            claim_id: ClaimId::new(20),
            artifact_id: ArtifactId::new(1),
            text: "Claim from evidence".to_string(),
            evidence_ids: Vec::new(),
        }),
        DomainInput::CreateCard(CreateCardInput {
            card_id: CardId::new(30),
            artifact_id: ArtifactId::new(1),
            title: "Summary".to_string(),
            body: "Summarize project notes".to_string(),
        }),
        DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: Some(ClaimId::new(20)),
            kind: EvidenceKind::FileSpan {
                path: "notes.txt".to_string(),
                range: ContentRange { start: 1, end: 2 },
                content_hash: "sha256:notes".to_string(),
                snapshot: None,
            },
            excerpt: "first chunk".to_string(),
            observed_at: LogicalTick::new(12),
        }),
        DomainInput::LinkEvidenceToClaim(LinkEvidenceToClaimInput {
            claim_id: ClaimId::new(20),
            evidence_id: EvidenceId::new(40),
        }),
        DomainInput::UserIntent(UserIntent {
            task_id: TaskId::new(50),
            title: "Summarize artifact".to_string(),
            priority: TaskPriority::Normal,
        }),
        DomainInput::ValidationCompleted(ValidationCompleted {
            claim_id: ClaimId::new(20),
            valid: true,
        }),
        DomainInput::ClockTick(LogicalTick::new(99)),
    ]
}

pub fn run_replay_once()
-> Result<(KernelState, Vec<DomainEventEnvelope>, Vec<MaestriaEffect>), DomainError> {
    replay_inputs(&sample_inputs())
}

pub fn file_span_kind() -> EvidenceKind {
    EvidenceKind::FileSpan {
        path: "notes.txt".to_string(),
        range: ContentRange { start: 1, end: 2 },
        content_hash: "sha256:notes".to_string(),
        snapshot: None,
    }
}
