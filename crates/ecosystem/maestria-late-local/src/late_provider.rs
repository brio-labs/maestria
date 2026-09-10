use std::sync::Arc;
use std::time::Duration;

use maestria_adapter_http::UreqJsonClient;
use maestria_domain::ContentHash;
use maestria_ports::{
    BoundedProviderTransport, LateInteractionProvider, MultiVectorDocument,
    MultiVectorDocumentRequest, MultiVectorIdentity, MultiVectorQuery, MultiVectorQueryRequest,
    MultiVectorSourceIdentity, MultiVectorToken, PortError, ProviderCallControl,
    ProviderDisclosure, ProviderEndpoint, RetentionPolicy,
};
use sha2::{Digest, Sha256};

use crate::dto::{EncodeRequest, EncodeResponse, WireInputKind};

const ENDPOINT_PATH: &str = "/v1/multivector";
const SCHEMA_VERSION: u16 = 1;
const MAX_REQUEST_BYTES: usize = 128 * 1024;

/// Provider-neutral local adapter for the supervised mLateOn sidecar.
#[derive(Clone)]
pub struct LocalHttpLateInteractionProvider {
    identity: MultiVectorIdentity,
    endpoint: ProviderEndpoint,
    transport: Arc<dyn BoundedProviderTransport>,
}

impl LocalHttpLateInteractionProvider {
    /// Creates a loopback-only provider using the bounded shared HTTP client.
    pub fn new(endpoint: &str, identity: MultiVectorIdentity) -> Result<Self, PortError> {
        let endpoint = ProviderEndpoint::loopback_http(endpoint, ENDPOINT_PATH)?;
        identity.validate()?;
        let transport = Arc::new(UreqJsonClient::new(
            endpoint.clone(),
            Duration::from_millis(250),
        ));
        Ok(Self {
            identity,
            endpoint,
            transport,
        })
    }

    /// Creates the adapter around a test or embedded bounded transport.
    pub fn with_transport(
        endpoint: &str,
        identity: MultiVectorIdentity,
        transport: Arc<dyn BoundedProviderTransport>,
    ) -> Result<Self, PortError> {
        let endpoint = ProviderEndpoint::loopback_http(endpoint, ENDPOINT_PATH)?;
        identity.validate()?;
        if transport.endpoint().as_str() != endpoint.as_str() {
            return Err(PortError::invalid_input(
                "late provider transport endpoint",
                "transport endpoint does not match adapter endpoint",
            ));
        }
        Ok(Self {
            identity,
            endpoint,
            transport,
        })
    }

    pub fn endpoint(&self) -> &ProviderEndpoint {
        &self.endpoint
    }

    fn identity_digest(&self) -> Result<String, PortError> {
        Ok(self.identity.digest()?.as_str().to_owned())
    }

    fn model(&self) -> &str {
        self.identity.fingerprint.base.model.as_str()
    }

    fn fingerprint(&self) -> &str {
        self.identity.fingerprint.base.artifact_hash.as_str()
    }

    fn encode_request(
        &self,
        request: &EncodeRequest<'_>,
        control: &dyn ProviderCallControl,
    ) -> Result<EncodeResponse, PortError> {
        if control.is_cancelled() {
            return Err(PortError::internal(
                "late provider request",
                "request cancelled before send",
            ));
        }
        if control.remaining_ms() == 0 {
            return Err(PortError::downstream(
                "late provider request",
                "request deadline elapsed before send",
            ));
        }
        let body = serde_json::to_vec(request).map_err(|error| {
            PortError::internal("encode late provider request", error.to_string())
        })?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(PortError::invalid_input(
                "late provider request",
                "request exceeds configured body limit",
            ));
        }
        let response = self.transport.post_bounded(body, control)?;
        if response.len() > control.max_response_bytes() {
            return Err(PortError::downstream(
                "late provider response",
                "response exceeds configured body limit",
            ));
        }
        serde_json::from_slice(&response).map_err(|error| {
            PortError::downstream("decode late provider response", error.to_string())
        })
    }

    fn validate_text(
        &self,
        text: &str,
        kind: WireInputKind,
        request_hash: &ContentHash,
    ) -> Result<(), PortError> {
        if text.trim().is_empty() {
            return Err(PortError::invalid_input(
                "late provider text",
                "text must not be blank",
            ));
        }
        let byte_limit = match kind {
            WireInputKind::Query => self.identity.fingerprint.query_policy.max_utf8_bytes,
            WireInputKind::Document => self.identity.fingerprint.document_policy.max_utf8_bytes,
        } as usize;
        if text.len() > byte_limit {
            return Err(PortError::invalid_input(
                "late provider text",
                "text exceeds profile UTF-8 byte limit",
            ));
        }
        let actual = hash_text(text)?;
        if &actual != request_hash {
            return Err(PortError::invalid_input(
                "late provider input hash",
                "request hash does not match exact text",
            ));
        }
        Ok(())
    }

    fn validate_response(
        &self,
        response: EncodeResponse,
        input_hash: &ContentHash,
        source: Option<&MultiVectorSourceIdentity>,
    ) -> Result<EncodeResponse, PortError> {
        if response.schema_version != SCHEMA_VERSION
            || response.model != self.model()
            || response.fingerprint != self.fingerprint()
            || response.profile_identity != self.fingerprint()
            || response.identity != self.identity_digest()?
            || response.input_hash != input_hash.as_str()
        {
            return Err(PortError::downstream(
                "late provider response",
                "response identity does not match request",
            ));
        }
        match source {
            Some(expected) if response.source.as_ref() == Some(expected) => {}
            Some(_) => {
                return Err(PortError::downstream(
                    "late provider response",
                    "response source binding does not match request",
                ));
            }
            None if response.source.is_some() => {
                return Err(PortError::downstream(
                    "late provider response",
                    "query response contains a document source binding",
                ));
            }
            None => {}
        }
        Ok(response)
    }

    fn tokens(response: EncodeResponse) -> Result<(u32, bool, Vec<MultiVectorToken>), PortError> {
        let mut tokens = Vec::with_capacity(response.vectors.len());
        for wire in response.vectors {
            tokens.push(MultiVectorToken::new(wire.position, wire.embedding)?);
        }
        Ok((response.original_token_count, response.truncated, tokens))
    }
}

impl LateInteractionProvider for LocalHttpLateInteractionProvider {
    fn disclosure(&self) -> Option<ProviderDisclosure> {
        Some(ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        })
    }

    fn identity(&self) -> Option<MultiVectorIdentity> {
        Some(self.identity.clone())
    }

    fn encode_query(
        &self,
        text: &str,
        request: &MultiVectorQueryRequest,
        control: &dyn ProviderCallControl,
    ) -> Result<MultiVectorQuery, PortError> {
        request.validate()?;
        if request.identity != self.identity {
            return Err(PortError::invalid_input(
                "late provider query identity",
                "request identity differs from provider identity",
            ));
        }
        self.validate_text(text, WireInputKind::Query, &request.input_hash)?;
        let identity = self.identity_digest()?;
        let response = self.encode_request(
            &EncodeRequest {
                schema_version: SCHEMA_VERSION,
                model: self.model(),
                fingerprint: self.fingerprint(),
                profile_identity: self.fingerprint(),
                identity: &identity,
                kind: WireInputKind::Query,
                input_hash: request.input_hash.as_str(),
                text,
                max_vectors: request.max_vectors,
                remaining_ms: control.remaining_ms(),
                source: None,
            },
            control,
        )?;
        let response = self.validate_response(response, &request.input_hash, None)?;
        let (original, truncated, tokens) = Self::tokens(response)?;
        MultiVectorQuery::new(
            self.identity.clone(),
            request.input_hash.clone(),
            original,
            truncated,
            tokens,
        )
    }

    fn encode_document(
        &self,
        text: &str,
        request: &MultiVectorDocumentRequest,
        control: &dyn ProviderCallControl,
    ) -> Result<MultiVectorDocument, PortError> {
        request.validate()?;
        if request.identity != self.identity {
            return Err(PortError::invalid_input(
                "late provider document identity",
                "request identity differs from provider identity",
            ));
        }
        self.validate_text(text, WireInputKind::Document, &request.input_hash)?;
        let identity = self.identity_digest()?;
        let response = self.encode_request(
            &EncodeRequest {
                schema_version: SCHEMA_VERSION,
                model: self.model(),
                fingerprint: self.fingerprint(),
                profile_identity: self.fingerprint(),
                identity: &identity,
                kind: WireInputKind::Document,
                input_hash: request.input_hash.as_str(),
                text,
                max_vectors: request.max_vectors,
                remaining_ms: control.remaining_ms(),
                source: Some(&request.source),
            },
            control,
        )?;
        let response =
            self.validate_response(response, &request.input_hash, Some(&request.source))?;
        let (original, truncated, tokens) = Self::tokens(response)?;
        MultiVectorDocument::new(
            self.identity.clone(),
            request.source.clone(),
            original,
            truncated,
            tokens,
        )
    }
}

fn hash_text(text: &str) -> Result<ContentHash, PortError> {
    let digest = Sha256::digest(text.as_bytes());
    let mut value = String::from("sha256:");
    for byte in digest {
        value.push_str(&format!("{byte:02x}"));
    }
    ContentHash::new(value)
        .map_err(|error| PortError::internal("hash late provider input", error.to_string()))
}

#[cfg(test)]
#[path = "late_provider_tests.rs"]
mod tests;
