use std::path::PathBuf;
use std::time::Duration;

use super::*;
use maestria_domain::ArtifactId;

pub fn assert_parser_round_trip(
    parser: &impl Parser,
    sample: &FileHandle,
    context: ParseContext,
) -> Result<(), Box<dyn std::error::Error>> {
    assert!(!parser.id().is_empty(), "parser id must not be empty");

    let supported = FileMetadata {
        path: sample.path.clone(),
        size: sample.bytes.len(),
        extension: sample
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase()),
    };
    assert!(
        parser.supports(&supported),
        "parser must support {:?}",
        supported.path
    );

    let unsupported = FileMetadata {
        path: PathBuf::from("archive.bin"),
        size: 5,
        extension: Some("bin".to_string()),
    };
    assert!(
        !parser.supports(&unsupported),
        "parser must not support {:?}",
        unsupported.path
    );

    let artifact_id = context.artifact_id;
    let next_artifact_id = ArtifactId::new(artifact_id.value().wrapping_add(1));
    let parsed = parser.parse(sample.clone(), context)?;
    assert_eq!(parsed.artifact_id, artifact_id);
    assert_eq!(parsed.status, ParseStatus::Parsed);
    assert!(
        !parsed.tree.nodes().is_empty(),
        "parsed tree must have at least one node"
    );

    assert!(matches!(
        parser.parse(
            FileHandle {
                path: sample.path.clone(),
                bytes: Vec::new(),
            },
            ParseContext {
                artifact_id: next_artifact_id,
            },
        ),
        Err(PortError::InvalidInputContext { .. })
    ));
    Ok(())
}

/// Shared contract assertion: a harness adapter round-trips a request.
///
/// # Cancellation
/// Dropping this future aborts the in-flight round trip; the adapter may
/// have started the underlying command, so the assertion result is lost but
/// no domain state is touched by this helper.
pub async fn assert_harness_adapter_round_trip(
    harness: &impl HarnessAdapter,
) -> Result<(), Box<dyn std::error::Error>> {
    let capabilities = harness.capabilities()?;
    assert!(capabilities.read_enabled);
    assert!(
        capabilities
            .command_classes
            .contains(&HarnessCommandClass::Shell)
    );

    let outcome = harness
        .execute(HarnessRequest {
            run_id: HarnessRunId::new(7),
            command: "echo ok".to_string(),
            working_directory: PathBuf::from("/tmp"),
            duration_budget: Duration::from_secs(1),
            class: HarnessCommandClass::Shell,
            readable_roots: vec![PathBuf::from("/tmp")],
            blocked_paths: vec![],
            blocked_patterns: vec![],
        })
        .await?;

    assert_eq!(outcome.run_id, HarnessRunId::new(7));
    assert_eq!(outcome.command, "echo ok");
    assert_eq!(outcome.exit_code, 0);
    assert!(!outcome.stdout.is_empty(), "stdout must not be empty");

    assert!(matches!(
        harness
            .execute(HarnessRequest {
                run_id: HarnessRunId::new(8),
                command: " ".to_string(),
                working_directory: PathBuf::from("/tmp"),
                duration_budget: Duration::from_secs(1),
                class: HarnessCommandClass::Shell,
                readable_roots: vec![PathBuf::from("/tmp")],
                blocked_paths: vec![],
                blocked_patterns: vec![],
            })
            .await,
        Err(error) if error.is_invalid_input()
    ));
    Ok(())
}

pub fn assert_web_fetcher_contract(
    fetcher: &impl super::WebFetcher,
    valid_url: &str,
    valid_html: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let fetch_res = fetcher.fetch(valid_url, valid_html.len().saturating_add(1))?;
    assert_eq!(fetch_res.url, valid_url, "URL must be preserved");
    assert_eq!(fetch_res.html, valid_html, "HTML must match");
    assert!(!fetch_res.html.is_empty(), "HTML should be non-empty");

    let empty_res = fetcher.fetch("", usize::MAX);
    assert!(
        matches!(empty_res, Err(super::PortError::InvalidInputContext { .. })),
        "Empty URLs must map to PortError::InvalidInput, got {:?}",
        empty_res
    );

    Ok(())
}

pub fn assert_embedding_provider_contract(
    provider: &impl EmbeddingProvider,
) -> Result<(), Box<dyn std::error::Error>> {
    let identity = provider
        .identity()
        .ok_or("embedding provider must disclose its identity")?;
    let response = provider.embed(EmbeddingRequest {
        text: "contract test input".to_string(),
        model: identity.fingerprint.model.as_str().to_string(),
        kind: EmbeddingInputKind::Document,
        identity: identity.clone(),
    })?;

    assert_eq!(response.identity, identity);
    assert_eq!(
        response.vector.len(),
        identity.fingerprint.dimensions as usize,
        "embedding dimensions must match the disclosed identity"
    );
    assert!(
        !response.vector.is_empty(),
        "embedding vector must not be empty"
    );
    assert!(
        !response.provider_id.is_empty(),
        "provider id must be disclosed"
    );
    assert!(!response.model.is_empty(), "model must be disclosed");
    assert!(
        !response.model_version.is_empty(),
        "model version must be disclosed"
    );
    Ok(())
}
