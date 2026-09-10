use maestria_domain::{
    ArtifactId, ArtifactVersionId, ContentHash, CorpusSnapshotId, EvidenceId, EvidenceSpan,
    IndexFingerprint, IndexGenerationId, LateInteractionAggregation, RealmId, RepresentationName,
    RetrievalModelFingerprint, TrustZone,
};
use serde::{Deserialize, Serialize};

use crate::PortError;

use super::{
    DEFAULT_MAX_DOCUMENT_VECTORS, DEFAULT_MAX_QUERY_VECTORS, MAX_MULTIVECTOR_DIMENSIONS,
    MULTIVECTOR_TEXT_V1,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiVectorInputKind {
    Query,
    Document,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiVectorSimilarity {
    DotProduct,
    Cosine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiVectorCompression {
    None,
    SymmetricInt8V1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MultiVectorTokenId(u32);

impl MultiVectorTokenId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u32 {
        self.0
    }
}

impl From<u32> for MultiVectorTokenId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultiVectorTokenPolicy {
    pub prefix_id: MultiVectorTokenId,
    pub prefix_text: String,
    pub native_token_limit: u32,
    pub max_vectors: u32,
    pub max_utf8_bytes: u32,
    pub truncate_right_after_prefix: bool,
    pub retain_special_tokens: bool,
    pub drop_masked_positions: bool,
    pub pad_token_id: MultiVectorTokenId,
    pub query_expansion: bool,
    pub lowercase: bool,
    pub punctuation_pruning: bool,
}

impl MultiVectorTokenPolicy {
    pub fn validate(&self, label: &'static str) -> Result<(), PortError> {
        if self.prefix_text.contains('\0') || self.prefix_text.chars().any(char::is_control) {
            return Err(PortError::invalid_input(
                "invalid multivector token policy",
                format!("{label} prefix contains a control character"),
            ));
        }
        if self.native_token_limit == 0
            || self.max_vectors == 0
            || self.max_utf8_bytes == 0
            || self.max_vectors > self.native_token_limit
        {
            return Err(PortError::invalid_input(
                "invalid multivector token policy",
                format!("{label} limits must be positive and bounded by native token limit"),
            ));
        }
        if !self.truncate_right_after_prefix {
            return Err(PortError::invalid_input(
                "unsupported multivector token policy",
                format!("{label} must truncate on the right after the prefix"),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultiVectorFingerprint {
    pub base: IndexFingerprint,
    pub tokenizer_hash: ContentHash,
    pub vocabulary_hash: ContentHash,
    pub vocabulary_size: u32,
    pub similarity: MultiVectorSimilarity,
    pub aggregation: LateInteractionAggregation,
    pub normalization_version: String,
    pub representation_compression: MultiVectorCompression,
    pub query_policy: MultiVectorTokenPolicy,
    pub document_policy: MultiVectorTokenPolicy,
}

impl MultiVectorFingerprint {
    pub fn validate(&self) -> Result<(), PortError> {
        if self.base.dimensions == 0 || self.base.dimensions as usize > MAX_MULTIVECTOR_DIMENSIONS {
            return Err(PortError::invalid_input(
                "invalid multivector dimensions",
                self.base.dimensions.to_string(),
            ));
        }
        validate_identity_string("normalization version", &self.normalization_version)?;
        if self.vocabulary_size == 0 {
            return Err(PortError::invalid_input(
                "invalid multivector vocabulary size",
                "vocabulary size must be positive",
            ));
        }
        self.query_policy.validate("query")?;
        self.document_policy.validate("document")?;
        if self.query_policy.max_vectors > DEFAULT_MAX_QUERY_VECTORS as u32
            || self.document_policy.max_vectors > DEFAULT_MAX_DOCUMENT_VECTORS as u32
        {
            return Err(PortError::invalid_input(
                "multivector vector count exceeds safety ceiling",
                "query/document vector count is too large",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let base = self.base.encode();
        let tokenizer = self.tokenizer_hash.as_str().to_owned();
        let vocabulary = self.vocabulary_hash.as_str().to_owned();
        let vocabulary_size = self.vocabulary_size.to_string();
        let similarity = format!("{:?}", self.similarity);
        let aggregation = format!("{:?}", self.aggregation);
        let compression = format!("{:?}", self.representation_compression);
        let query_policy = token_policy_encoding(&self.query_policy);
        let document_policy = token_policy_encoding(&self.document_policy);
        length_prefixed(vec![
            base.as_bytes(),
            tokenizer.as_bytes(),
            vocabulary.as_bytes(),
            vocabulary_size.as_bytes(),
            similarity.as_bytes(),
            aggregation.as_bytes(),
            self.normalization_version.as_bytes(),
            compression.as_bytes(),
            &query_policy,
            &document_policy,
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultiVectorIdentity {
    pub representation: RepresentationName,
    pub fingerprint: MultiVectorFingerprint,
    pub generation_id: IndexGenerationId,
    pub corpus_snapshot: CorpusSnapshotId,
    pub realm: RealmId,
    pub trust_zone: TrustZone,
}

impl MultiVectorIdentity {
    pub fn validate(&self) -> Result<(), PortError> {
        if self.representation.as_str() != MULTIVECTOR_TEXT_V1 {
            return Err(PortError::invalid_input(
                "invalid multivector representation",
                self.representation.as_str(),
            ));
        }
        if self.generation_id.value() == 0 || self.corpus_snapshot.value() == 0 {
            return Err(PortError::invalid_input(
                "invalid multivector identity",
                "generation and corpus identifiers must be nonzero",
            ));
        }
        if self.realm.as_str().bytes().all(|byte| byte == b'0') {
            return Err(PortError::invalid_input(
                "invalid multivector realm",
                "realm identifier must not be all zero",
            ));
        }
        self.fingerprint.validate()
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let fingerprint = self.fingerprint.canonical_bytes();
        let generation = self.generation_id.value().to_string();
        let corpus_snapshot = self.corpus_snapshot.value().to_string();
        let trust_zone = format!("{:?}", self.trust_zone);
        length_prefixed(vec![
            self.representation.as_str().as_bytes(),
            &fingerprint,
            generation.as_bytes(),
            corpus_snapshot.as_bytes(),
            self.realm.as_str().as_bytes(),
            trust_zone.as_bytes(),
        ])
    }

    pub fn digest(&self) -> Result<ContentHash, PortError> {
        self.validate()?;
        content_hash(&self.canonical_bytes())
    }

    pub fn retrieval_fingerprint(&self) -> Result<RetrievalModelFingerprint, PortError> {
        let digest = self.digest()?;
        RetrievalModelFingerprint::new(digest.as_str().to_owned()).map_err(|error| {
            PortError::invalid_input("multivector retrieval fingerprint", error.to_string())
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultiVectorSourceIdentity {
    pub evidence_id: EvidenceId,
    pub artifact_id: ArtifactId,
    pub version_id: ArtifactVersionId,
    pub source_snapshot_hash: ContentHash,
    pub span: EvidenceSpan,
    pub source_representation_hash: ContentHash,
}

impl MultiVectorSourceIdentity {
    pub fn validate(&self) -> Result<(), PortError> {
        if self.evidence_id.value() == 0
            || self.artifact_id.value() == 0
            || self.version_id.value() == 0
        {
            return Err(PortError::invalid_input(
                "invalid multivector source identity",
                "source identifiers must be nonzero",
            ));
        }
        Ok(())
    }

    pub fn representation_hash(
        excerpt: &str,
        evidence_id: EvidenceId,
        artifact_id: ArtifactId,
        version_id: ArtifactVersionId,
        source_snapshot_hash: &ContentHash,
        span: &EvidenceSpan,
    ) -> Result<ContentHash, PortError> {
        let span = span.canonical_bytes();
        let bytes = length_prefixed(vec![
            b"late-interaction-source-v1",
            excerpt.as_bytes(),
            evidence_id.value().to_string().as_bytes(),
            artifact_id.value().to_string().as_bytes(),
            version_id.value().to_string().as_bytes(),
            source_snapshot_hash.as_str().as_bytes(),
            &span,
        ]);
        content_hash(&bytes)
    }
}
fn validate_identity_string(label: &'static str, value: &str) -> Result<(), PortError> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(PortError::invalid_input(
            "invalid multivector identity string",
            format!("{label} is blank, too long, or contains a control character"),
        ));
    }
    Ok(())
}

fn token_policy_encoding(policy: &MultiVectorTokenPolicy) -> Vec<u8> {
    let prefix_id = policy.prefix_id.value().to_string();
    let native_token_limit = policy.native_token_limit.to_string();
    let max_vectors = policy.max_vectors.to_string();
    let max_utf8_bytes = policy.max_utf8_bytes.to_string();
    let pad_token_id = policy.pad_token_id.value().to_string();
    let truncate_right_after_prefix = policy.truncate_right_after_prefix.to_string();
    let retain_special_tokens = policy.retain_special_tokens.to_string();
    let drop_masked_positions = policy.drop_masked_positions.to_string();
    let query_expansion = policy.query_expansion.to_string();
    let lowercase = policy.lowercase.to_string();
    let punctuation_pruning = policy.punctuation_pruning.to_string();
    length_prefixed(vec![
        prefix_id.as_bytes(),
        policy.prefix_text.as_bytes(),
        native_token_limit.as_bytes(),
        max_vectors.as_bytes(),
        max_utf8_bytes.as_bytes(),
        truncate_right_after_prefix.as_bytes(),
        retain_special_tokens.as_bytes(),
        drop_masked_positions.as_bytes(),
        pad_token_id.as_bytes(),
        query_expansion.as_bytes(),
        lowercase.as_bytes(),
        punctuation_pruning.as_bytes(),
    ])
}

fn length_prefixed(parts: Vec<&[u8]>) -> Vec<u8> {
    let mut output = Vec::new();
    for part in parts {
        output.extend_from_slice(&(part.len() as u64).to_be_bytes());
        output.extend_from_slice(part);
    }
    output
}

fn content_hash(bytes: &[u8]) -> Result<ContentHash, PortError> {
    ContentHash::new(maestria_domain::content_hash(bytes))
        .map_err(|error| PortError::internal("create multivector content hash", error.to_string()))
}
