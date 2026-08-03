use maestria_domain::BlobId;

use crate::{VisualEmbeddingProvider, VisualEmbeddingRequest, VisualSource};

const VISUAL_REPRESENTATION: &str = "visual_page_v1";

/// Shared behavioral contract every concrete `VisualEmbeddingProvider` must
/// satisfy.
///
/// Concrete adapters execute this suite in their own test modules alongside
/// adapter-specific boundary tests (Rule 25). The contract covers identity
/// disclosure, query embedding, and page/region source embedding with
/// identity-preserving, dimension-correct responses.
pub fn assert_visual_embedding_provider_contract(
    provider: &impl VisualEmbeddingProvider,
) -> Result<(), Box<dyn std::error::Error>> {
    let identity = provider
        .identity()
        .ok_or("visual provider must disclose its identity")?;
    assert_eq!(
        identity.representation,
        maestria_domain::RepresentationName::new(VISUAL_REPRESENTATION),
        "visual provider identity representation must be {VISUAL_REPRESENTATION}"
    );
    assert!(
        identity.fingerprint.dimensions > 0,
        "visual provider identity dimensions must be positive"
    );
    assert!(
        !identity.fingerprint.model.as_str().is_empty(),
        "visual provider identity model must be disclosed"
    );
    assert!(
        !identity.fingerprint.provider.as_str().is_empty(),
        "visual provider identity provider name must be disclosed"
    );

    let disclosure = provider.disclosure();
    let response = provider.embed_query("contract test visual query", identity.clone())?;
    assert_visual_response(&response, &identity, &disclosure, "query")?;

    let page = provider.embed_source(VisualEmbeddingRequest {
        source: VisualSource::Page {
            blob: BlobId::new(1),
            page_start: 1,
            page_end: 2,
        },
        bytes: b"contract-test-page-bytes".to_vec(),
        identity: identity.clone(),
    })?;
    assert_visual_response(&page, &identity, &disclosure, "page")?;

    let region = provider.embed_source(VisualEmbeddingRequest {
        source: VisualSource::Region {
            blob: BlobId::new(2),
            page: 1,
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        },
        bytes: b"contract-test-region-bytes".to_vec(),
        identity: identity.clone(),
    })?;
    assert_visual_response(&region, &identity, &disclosure, "region")?;

    // Identity, disclosure, and response shape stay stable across calls.
    let second = provider.embed_query("contract test visual query", identity.clone())?;
    assert_eq!(second.identity, identity);
    assert_eq!(second.disclosure, disclosure);
    assert_eq!(
        second.vector.len(),
        identity.fingerprint.dimensions as usize
    );
    Ok(())
}

fn assert_visual_response(
    response: &crate::EmbeddingResponse,
    identity: &crate::EmbeddingIdentity,
    disclosure: &crate::ProviderDisclosure,
    kind: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        response.identity, *identity,
        "visual provider {kind} response must preserve the request identity"
    );
    assert_eq!(
        response.disclosure, *disclosure,
        "visual provider {kind} response must preserve the provider disclosure"
    );
    assert_eq!(
        response.vector.len(),
        identity.fingerprint.dimensions as usize,
        "visual provider {kind} embedding dimensions must match the disclosed identity"
    );
    assert!(
        response.vector.iter().all(|value| value.is_finite()),
        "visual provider {kind} embedding must contain only finite values"
    );
    assert!(
        !response.provider_id.is_empty(),
        "visual provider {kind} response must disclose its provider id"
    );
    assert!(
        !response.model.is_empty(),
        "visual provider {kind} response must disclose its model"
    );
    assert!(
        !response.model_version.is_empty(),
        "visual provider {kind} response must disclose its model version"
    );
    Ok(())
}
