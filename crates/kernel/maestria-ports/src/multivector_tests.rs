use super::*;
use crate::PortError;
use maestria_domain::{
    ContentHash, CorpusSnapshotId, IndexFingerprint, IndexGenerationId, LateInteractionAggregation,
    RealmId, TrustZone,
};

fn identity() -> Result<MultiVectorIdentity, Box<dyn std::error::Error>> {
    let hash = ContentHash::new(format!("sha256:{}", "a".repeat(64)))?;
    let realm = RealmId::try_from("b".repeat(64))
        .map_err(|error| PortError::invalid_input("test realm", error.to_string()))?;
    Ok(MultiVectorIdentity {
        representation: MULTIVECTOR_TEXT_V1.into(),
        fingerprint: MultiVectorFingerprint {
            base: IndexFingerprint {
                provider: "mlateon-onnx".into(),
                model: "mlateon".into(),
                revision: "local".into(),
                artifact_hash: hash.clone(),
                dimensions: 2,
                quantization: "onnx_int8".into(),
                query_template_hash: hash.clone(),
                document_template_hash: hash.clone(),
                preprocessing_version: "test".into(),
            },
            tokenizer_hash: hash.clone(),
            vocabulary_hash: hash,
            vocabulary_size: 4,
            similarity: MultiVectorSimilarity::Cosine,
            aggregation: LateInteractionAggregation::QueryTokenMaxThenSum,
            normalization_version: "L2PerTokenV1".to_string(),
            representation_compression: MultiVectorCompression::None,
            query_policy: MultiVectorTokenPolicy {
                prefix_id: 1.into(),
                prefix_text: "[Q] ".to_string(),
                native_token_limit: 8192,
                max_vectors: 2,
                max_utf8_bytes: 8192,
                truncate_right_after_prefix: true,
                retain_special_tokens: true,
                drop_masked_positions: true,
                pad_token_id: 4.into(),
                query_expansion: false,
                lowercase: false,
                punctuation_pruning: false,
            },
            document_policy: MultiVectorTokenPolicy {
                prefix_id: 2.into(),
                prefix_text: "[D] ".to_string(),
                native_token_limit: 8192,
                max_vectors: 2,
                max_utf8_bytes: 65536,
                truncate_right_after_prefix: true,
                retain_special_tokens: true,
                drop_masked_positions: true,
                pad_token_id: 4.into(),
                query_expansion: false,
                lowercase: false,
                punctuation_pruning: false,
            },
        },
        generation_id: IndexGenerationId::new(1),
        corpus_snapshot: CorpusSnapshotId::new(1),
        realm,
        trust_zone: TrustZone::Verified,
    })
}

#[test]
fn rejects_duplicate_positions_and_wrong_dimension() -> Result<(), Box<dyn std::error::Error>> {
    let identity = identity()?;
    let hash = ContentHash::new(format!("sha256:{}", "c".repeat(64)))?;
    let first = MultiVectorToken::new(0, vec![1.0, 0.0])?;
    let second = MultiVectorToken::new(0, vec![0.0, 1.0])?;
    let result = MultiVectorQuery::new(identity, hash, 2, false, vec![first, second]);
    assert!(result.is_err());
    Ok(())
}

#[test]
fn rejects_original_token_count_above_native_limit() -> Result<(), Box<dyn std::error::Error>> {
    let identity = identity()?;
    let hash = ContentHash::new(format!("sha256:{}", "d".repeat(64)))?;
    let token = MultiVectorToken::new(0, vec![1.0, 0.0])?;
    assert!(MultiVectorQuery::new(identity, hash, 8_193, false, vec![token]).is_err());
    Ok(())
}
