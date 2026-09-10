use super::*;
use maestria_domain::{
    CorpusSnapshotId, IndexFingerprint, IndexGenerationId, LateInteractionAggregation, RealmId,
    RepresentationName,
};
use maestria_ports::{
    BoundedProviderTransport, MultiVectorCompression, MultiVectorFingerprint,
    MultiVectorSimilarity, MultiVectorTokenPolicy, ProviderTransport,
};
use std::sync::Mutex;

struct Control;
impl ProviderCallControl for Control {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn remaining_ms(&self) -> u32 {
        250
    }
    fn max_response_bytes(&self) -> usize {
        2 * 1024 * 1024
    }
}

struct Transport {
    endpoint: ProviderEndpoint,
    response: Mutex<Vec<u8>>,
}
impl ProviderTransport for Transport {
    fn endpoint(&self) -> &ProviderEndpoint {
        &self.endpoint
    }
    fn disclosure(&self) -> &ProviderDisclosure {
        static DISCLOSURE: std::sync::LazyLock<ProviderDisclosure> =
            std::sync::LazyLock::new(|| ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            });
        &DISCLOSURE
    }
    fn post(&self, _body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        self.response
            .lock()
            .map(|response| response.clone())
            .map_err(|_| PortError::internal("test transport lock", "poisoned mutex"))
    }
}
impl BoundedProviderTransport for Transport {
    fn post_bounded(
        &self,
        _body: Vec<u8>,
        _control: &dyn ProviderCallControl,
    ) -> Result<Vec<u8>, PortError> {
        self.response
            .lock()
            .map(|response| response.clone())
            .map_err(|_| PortError::internal("test transport lock", "poisoned mutex"))
    }
}

fn identity() -> Result<MultiVectorIdentity, Box<dyn std::error::Error>> {
    let hash = ContentHash::new(format!("sha256:{}", "a".repeat(64)))?;
    let realm = RealmId::try_from("b".repeat(64))?;
    let policy = |prefix_id, prefix_text, max_vectors, max_utf8_bytes| MultiVectorTokenPolicy {
        prefix_id,
        prefix_text,
        native_token_limit: 8192,
        max_vectors,
        max_utf8_bytes,
        truncate_right_after_prefix: true,
        retain_special_tokens: true,
        drop_masked_positions: true,
        pad_token_id: 4.into(),
        query_expansion: false,
        lowercase: false,
        punctuation_pruning: false,
    };
    Ok(MultiVectorIdentity {
        representation: RepresentationName::new("multivector_text_v1"),
        fingerprint: MultiVectorFingerprint {
            base: IndexFingerprint {
                provider: "mlateon-onnx".into(),
                model: "model".into(),
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
            vocabulary_size: 2,
            similarity: MultiVectorSimilarity::Cosine,
            aggregation: LateInteractionAggregation::QueryTokenMaxThenSum,
            normalization_version: "L2PerTokenV1".into(),
            representation_compression: MultiVectorCompression::None,
            query_policy: policy(1.into(), "[Q] ".into(), 2, 8192),
            document_policy: policy(2.into(), "[D] ".into(), 2, 65536),
        },
        generation_id: IndexGenerationId::new(1),
        corpus_snapshot: CorpusSnapshotId::new(1),
        realm,
        trust_zone: maestria_domain::TrustZone::Verified,
    })
}

fn response_value(
    identity: &MultiVectorIdentity,
    input_hash: &ContentHash,
    response_identity: &str,
    include_profile_identity: bool,
) -> serde_json::Value {
    let mut vector = serde_json::Map::new();
    vector.insert("position".into(), serde_json::Value::from(0));
    vector.insert(
        "embedding".into(),
        serde_json::Value::Array(vec![
            serde_json::Value::from(1.0),
            serde_json::Value::from(0.0),
        ]),
    );
    let mut response = serde_json::Map::new();
    response.insert("schema_version".into(), serde_json::Value::from(1));
    response.insert("model".into(), serde_json::Value::from("model"));
    response.insert(
        "fingerprint".into(),
        serde_json::Value::from(identity.fingerprint.base.artifact_hash.as_str()),
    );
    if include_profile_identity {
        response.insert(
            "profile_identity".into(),
            serde_json::Value::from(identity.fingerprint.base.artifact_hash.as_str()),
        );
    }
    response.insert(
        "identity".into(),
        serde_json::Value::from(response_identity),
    );
    response.insert(
        "input_hash".into(),
        serde_json::Value::from(input_hash.as_str()),
    );
    response.insert("original_token_count".into(), serde_json::Value::from(1));
    response.insert("truncated".into(), serde_json::Value::from(false));
    response.insert(
        "vectors".into(),
        serde_json::Value::Array(vec![serde_json::Value::Object(vector)]),
    );
    serde_json::Value::Object(response)
}

#[test]
fn rejects_response_identity_drift_before_value_construction()
-> Result<(), Box<dyn std::error::Error>> {
    let endpoint =
        ProviderEndpoint::loopback_http("http://127.0.0.1:8093/v1/multivector", "/v1/multivector")?;
    let text_hash = hash_text("hello")?;
    let identity = identity()?;
    let response = response_value(&identity, &text_hash, "sha256:bad", false);
    let transport = Arc::new(Transport {
        endpoint: endpoint.clone(),
        response: Mutex::new(serde_json::to_vec(&response)?),
    });
    let provider = LocalHttpLateInteractionProvider::with_transport(
        "http://127.0.0.1:8093/v1/multivector",
        identity.clone(),
        transport,
    )?;
    let request = MultiVectorQueryRequest {
        identity: identity.clone(),
        input_hash: text_hash,
        max_vectors: 2,
    };
    assert!(provider.encode_query("hello", &request, &Control).is_err());
    Ok(())
}

#[test]
fn accepts_profile_identity_and_complete_identity_echo() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint =
        ProviderEndpoint::loopback_http("http://127.0.0.1:8093/v1/multivector", "/v1/multivector")?;
    let text_hash = hash_text("hello")?;
    let identity = identity()?;
    let identity_digest = identity.digest()?;
    let response = response_value(&identity, &text_hash, identity_digest.as_str(), true);
    let transport = Arc::new(Transport {
        endpoint: endpoint.clone(),
        response: Mutex::new(serde_json::to_vec(&response)?),
    });
    let provider = LocalHttpLateInteractionProvider::with_transport(
        "http://127.0.0.1:8093/v1/multivector",
        identity.clone(),
        transport,
    )?;
    let request = MultiVectorQueryRequest {
        identity,
        input_hash: text_hash,
        max_vectors: 2,
    };
    let query = provider.encode_query("hello", &request, &Control)?;
    assert_eq!(query.tokens().len(), 1);
    Ok(())
}
