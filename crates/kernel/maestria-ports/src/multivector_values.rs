use std::cmp::Ordering;

use maestria_domain::ContentHash;

use crate::{PortError, ProviderDisclosure, ProviderTransport};

use super::{
    MAX_MULTIVECTOR_DIMENSIONS,
    identity::{MultiVectorIdentity, MultiVectorInputKind, MultiVectorSourceIdentity},
};
#[derive(Debug, Clone, PartialEq)]
pub struct MultiVectorToken {
    position: u32,
    embedding: Vec<f32>,
}

impl MultiVectorToken {
    pub fn new(position: u32, embedding: Vec<f32>) -> Result<Self, PortError> {
        if embedding.is_empty() || embedding.len() > MAX_MULTIVECTOR_DIMENSIONS {
            return Err(PortError::invalid_input(
                "invalid multivector token dimension",
                embedding.len().to_string(),
            ));
        }
        if embedding.iter().any(|value| !value.is_finite()) {
            return Err(PortError::invalid_input(
                "invalid multivector token",
                "embedding contains a non-finite coordinate",
            ));
        }
        Ok(Self {
            position,
            embedding,
        })
    }

    pub fn position(&self) -> u32 {
        self.position
    }

    pub fn embedding(&self) -> &[f32] {
        &self.embedding
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultiVectorQuery {
    identity: MultiVectorIdentity,
    input_hash: ContentHash,
    original_token_count: u32,
    truncated: bool,
    tokens: Vec<MultiVectorToken>,
}

impl MultiVectorQuery {
    pub fn new(
        identity: MultiVectorIdentity,
        input_hash: ContentHash,
        original_token_count: u32,
        truncated: bool,
        tokens: Vec<MultiVectorToken>,
    ) -> Result<Self, PortError> {
        validate_token_set(
            &identity,
            MultiVectorInputKind::Query,
            original_token_count,
            &tokens,
        )?;
        Ok(Self {
            identity,
            input_hash,
            original_token_count,
            truncated,
            tokens,
        })
    }

    pub fn identity(&self) -> &MultiVectorIdentity {
        &self.identity
    }

    pub fn input_hash(&self) -> &ContentHash {
        &self.input_hash
    }

    pub fn original_token_count(&self) -> u32 {
        self.original_token_count
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn tokens(&self) -> &[MultiVectorToken] {
        &self.tokens
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct MultiVectorDocument {
    identity: MultiVectorIdentity,
    source: MultiVectorSourceIdentity,
    original_token_count: u32,
    truncated: bool,
    tokens: Vec<MultiVectorToken>,
}

impl MultiVectorDocument {
    pub fn new(
        identity: MultiVectorIdentity,
        source: MultiVectorSourceIdentity,
        original_token_count: u32,
        truncated: bool,
        tokens: Vec<MultiVectorToken>,
    ) -> Result<Self, PortError> {
        source.validate()?;
        validate_token_set(
            &identity,
            MultiVectorInputKind::Document,
            original_token_count,
            &tokens,
        )?;
        Ok(Self {
            identity,
            source,
            original_token_count,
            truncated,
            tokens,
        })
    }

    pub fn identity(&self) -> &MultiVectorIdentity {
        &self.identity
    }

    pub fn source(&self) -> &MultiVectorSourceIdentity {
        &self.source
    }

    pub fn original_token_count(&self) -> u32 {
        self.original_token_count
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn tokens(&self) -> &[MultiVectorToken] {
        &self.tokens
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiVectorQueryRequest {
    pub identity: MultiVectorIdentity,
    pub input_hash: ContentHash,
    pub max_vectors: u32,
}

impl MultiVectorQueryRequest {
    pub fn validate(&self) -> Result<(), PortError> {
        self.identity.validate()?;
        if self.max_vectors != self.identity.fingerprint.query_policy.max_vectors {
            return Err(PortError::invalid_input(
                "invalid multivector query request",
                "max_vectors does not match identity",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiVectorDocumentRequest {
    pub identity: MultiVectorIdentity,
    pub source: MultiVectorSourceIdentity,
    pub input_hash: ContentHash,
    pub max_vectors: u32,
}

impl MultiVectorDocumentRequest {
    pub fn validate(&self) -> Result<(), PortError> {
        self.identity.validate()?;
        self.source.validate()?;
        if self.max_vectors != self.identity.fingerprint.document_policy.max_vectors {
            return Err(PortError::invalid_input(
                "invalid multivector document request",
                "max_vectors does not match identity",
            ));
        }
        Ok(())
    }
}

pub trait ProviderCallControl: Send + Sync {
    fn is_cancelled(&self) -> bool;
    fn remaining_ms(&self) -> u32;
    fn max_response_bytes(&self) -> usize;
}

pub trait BoundedProviderTransport: ProviderTransport {
    fn post_bounded(
        &self,
        body: Vec<u8>,
        control: &dyn ProviderCallControl,
    ) -> Result<Vec<u8>, PortError>;
}

pub trait LateInteractionProvider: Send + Sync {
    fn disclosure(&self) -> Option<ProviderDisclosure>;
    fn identity(&self) -> Option<MultiVectorIdentity>;
    fn encode_query(
        &self,
        text: &str,
        request: &MultiVectorQueryRequest,
        control: &dyn ProviderCallControl,
    ) -> Result<MultiVectorQuery, PortError>;
    fn encode_document(
        &self,
        text: &str,
        request: &MultiVectorDocumentRequest,
        control: &dyn ProviderCallControl,
    ) -> Result<MultiVectorDocument, PortError>;
}

pub trait LateInteractionScorer: Send + Sync {
    fn score(
        &self,
        query: &MultiVectorQuery,
        document: &MultiVectorDocument,
        control: &dyn ProviderCallControl,
    ) -> Result<LateInteractionScoringResult, PortError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LateInteractionContribution {
    pub query_position: u32,
    pub document_position: u32,
    pub similarity_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LateInteractionScoringResult {
    pub aggregate_micros: i64,
    pub denominator: i64,
    pub scale: &'static str,
    pub total_query_tokens: u32,
    pub retained_contribution_count: u32,
    pub omitted_contribution_count: u32,
    pub omitted_contribution_sum_micros: i64,
    pub contributions: Vec<LateInteractionContribution>,
}

fn validate_token_set(
    identity: &MultiVectorIdentity,
    kind: MultiVectorInputKind,
    original_token_count: u32,
    tokens: &[MultiVectorToken],
) -> Result<(), PortError> {
    identity.validate()?;
    if original_token_count == 0 || tokens.is_empty() {
        return Err(PortError::invalid_input(
            "invalid multivector token set",
            "token set must not be empty",
        ));
    }
    let (max_vectors, native_token_limit) = match kind {
        MultiVectorInputKind::Query => (
            identity.fingerprint.query_policy.max_vectors,
            identity.fingerprint.query_policy.native_token_limit,
        ),
        MultiVectorInputKind::Document => (
            identity.fingerprint.document_policy.max_vectors,
            identity.fingerprint.document_policy.native_token_limit,
        ),
    };
    if original_token_count > native_token_limit {
        return Err(PortError::invalid_input(
            "multivector token count exceeds native limit",
            original_token_count.to_string(),
        ));
    }
    let limit = max_vectors as usize;
    if tokens.len() > limit {
        return Err(PortError::invalid_input(
            "multivector token count exceeds request cap",
            tokens.len().to_string(),
        ));
    }
    let dimensions = identity.fingerprint.base.dimensions as usize;
    let bytes = tokens
        .len()
        .checked_mul(dimensions)
        .and_then(|value| value.checked_mul(std::mem::size_of::<f32>()))
        .ok_or_else(|| PortError::invalid_input("multivector byte count overflow", "token set"))?;
    if bytes > 32 * 1024 * 1024 {
        return Err(PortError::invalid_input(
            "multivector representation exceeds working budget",
            bytes.to_string(),
        ));
    }
    let mut previous = None;
    for token in tokens {
        if token.embedding.len() != dimensions {
            return Err(PortError::invalid_input(
                "multivector dimension mismatch",
                format!("expected {dimensions}, got {}", token.embedding.len()),
            ));
        }
        if token.position >= original_token_count {
            return Err(PortError::invalid_input(
                "multivector token position out of range",
                token.position.to_string(),
            ));
        }
        if previous.is_some_and(|position| position >= token.position) {
            return Err(PortError::invalid_input(
                "multivector token positions are not strictly increasing",
                token.position.to_string(),
            ));
        }
        previous = Some(token.position);
    }
    Ok(())
}
/// Stable comparison for contribution summaries.
pub fn contribution_order(
    left: &LateInteractionContribution,
    right: &LateInteractionContribution,
) -> Ordering {
    right
        .similarity_micros
        .unsigned_abs()
        .cmp(&left.similarity_micros.unsigned_abs())
        .then_with(|| left.query_position.cmp(&right.query_position))
        .then_with(|| left.document_position.cmp(&right.document_position))
}
