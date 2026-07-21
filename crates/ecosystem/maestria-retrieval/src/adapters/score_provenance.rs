use std::collections::BTreeMap;

use maestria_domain::{
    RepresentationName, RetrievalLaneScore, RetrievalModelFingerprint, RetrievalRawRank,
    RetrievalScoreFingerprint, RetrievalScoreKind, RetrievalScoreScale, RetrievalScoreSet,
};
use maestria_ports::{EmbeddingIdentity, SparseIdentity};

use crate::types::{RetrievalError, RetrieverDescriptor};

pub(super) fn lexical_score(
    descriptor: &RetrieverDescriptor,
    raw_score: u32,
    raw_rank: u32,
) -> Result<RetrievalScoreSet, RetrievalError> {
    score_set(
        RetrievalScoreKind::LexicalBm25,
        i64::from(raw_score),
        raw_rank,
        RetrievalScoreScale::unbounded("bm25"),
        descriptor.representation.clone(),
        descriptor_fingerprint(descriptor, "lexical_bm25", BTreeMap::new())?,
    )
}

pub(super) fn dense_score(
    descriptor: &RetrieverDescriptor,
    raw_score_micros: u32,
    raw_rank: u32,
    identity: &EmbeddingIdentity,
    scale_name: &str,
) -> Result<RetrievalScoreSet, RetrievalError> {
    if identity.generation_id != descriptor.generation {
        return Err(RetrievalError::Internal(
            "dense score identity generation does not match its retriever descriptor".to_string(),
        ));
    }
    if identity.representation != descriptor.representation {
        return Err(RetrievalError::Internal(
            "dense score identity representation does not match its retriever descriptor"
                .to_string(),
        ));
    }
    let fingerprint = &identity.fingerprint;
    let components = BTreeMap::from([
        ("provider".to_string(), fingerprint.provider.clone()),
        ("model".to_string(), fingerprint.model.clone()),
        ("revision".to_string(), fingerprint.revision.clone()),
        (
            "artifact_hash".to_string(),
            fingerprint.artifact_hash.as_str().to_string(),
        ),
        ("dimensions".to_string(), fingerprint.dimensions.to_string()),
        ("quantization".to_string(), fingerprint.quantization.clone()),
        (
            "query_template_hash".to_string(),
            fingerprint.query_template_hash.clone(),
        ),
        (
            "document_template_hash".to_string(),
            fingerprint.document_template_hash.clone(),
        ),
        (
            "preprocessing_version".to_string(),
            fingerprint.preprocessing_version.clone(),
        ),
        (
            "generation".to_string(),
            identity.generation_id.value().to_string(),
        ),
        (
            "representation".to_string(),
            identity.representation.0.clone(),
        ),
    ]);
    let fingerprint_id = RetrievalModelFingerprint::new(format!(
        "dense:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        fingerprint.provider,
        fingerprint.model,
        fingerprint.revision,
        fingerprint.artifact_hash.as_str(),
        fingerprint.dimensions,
        fingerprint.quantization,
        fingerprint.query_template_hash,
        fingerprint.document_template_hash,
        fingerprint.preprocessing_version,
    ))?;
    score_set(
        RetrievalScoreKind::DenseSimilarity,
        i64::from(raw_score_micros),
        raw_rank,
        RetrievalScoreScale::bounded_fixed_point(scale_name, 1_000_000, 0, 1_000_000),
        identity.representation.clone(),
        RetrievalScoreFingerprint::new(fingerprint_id, components),
    )
}

pub(super) fn learned_sparse_score(
    identity: &SparseIdentity,
    fingerprint_id: RetrievalModelFingerprint,
    raw_score_micros: u32,
    raw_rank: u32,
) -> Result<RetrievalScoreSet, RetrievalError> {
    let fingerprint = &identity.fingerprint;
    let components = BTreeMap::from([
        ("provider".to_string(), fingerprint.provider.clone()),
        ("model".to_string(), fingerprint.model.clone()),
        ("revision".to_string(), fingerprint.revision.clone()),
        (
            "artifact_hash".to_string(),
            fingerprint.artifact_hash.as_str().to_string(),
        ),
        (
            "tokenizer_hash".to_string(),
            fingerprint.tokenizer_hash.as_str().to_string(),
        ),
        (
            "vocabulary_hash".to_string(),
            fingerprint.vocabulary_hash.as_str().to_string(),
        ),
        (
            "vocabulary_size".to_string(),
            fingerprint.vocabulary_size.to_string(),
        ),
        (
            "term_namespace".to_string(),
            fingerprint.term_namespace.clone(),
        ),
        (
            "query_template_hash".to_string(),
            fingerprint.query_template_hash.as_str().to_string(),
        ),
        (
            "document_template_hash".to_string(),
            fingerprint.document_template_hash.as_str().to_string(),
        ),
        (
            "preprocessing_version".to_string(),
            fingerprint.preprocessing_version.clone(),
        ),
        (
            "weighting_version".to_string(),
            fingerprint.weighting_version.clone(),
        ),
        ("quantization".to_string(), fingerprint.quantization.clone()),
        (
            "pruning_threshold".to_string(),
            fingerprint.pruning_threshold.to_string(),
        ),
        ("max_terms".to_string(), fingerprint.max_terms.to_string()),
        (
            "generation".to_string(),
            identity.generation_id.value().to_string(),
        ),
        (
            "corpus_snapshot".to_string(),
            identity.corpus_snapshot.value().to_string(),
        ),
        (
            "representation".to_string(),
            identity.representation.0.clone(),
        ),
    ]);
    score_set(
        RetrievalScoreKind::LearnedSparse,
        i64::from(raw_score_micros),
        raw_rank,
        RetrievalScoreScale::fixed_point("learned_sparse_dot_product_micros", 1_000_000),
        identity.representation.clone(),
        RetrievalScoreFingerprint::new(fingerprint_id, components),
    )
}

pub(super) fn specialized_score(
    descriptor: &RetrieverDescriptor,
    route: &str,
    raw_score: u32,
    raw_rank: u32,
    scale_name: &str,
    components: BTreeMap<String, String>,
) -> Result<RetrievalScoreSet, RetrievalError> {
    score_set(
        RetrievalScoreKind::SpecializedRetrieval {
            route: route.to_string(),
        },
        i64::from(raw_score),
        raw_rank,
        RetrievalScoreScale::rank_derived(scale_name),
        descriptor.representation.clone(),
        descriptor_fingerprint(descriptor, route, components)?,
    )
}

pub(super) fn graph_score(
    raw_score: u32,
    raw_rank: u32,
    seed_rank: u32,
    depth: usize,
    confidence_milli: u16,
) -> Result<RetrievalScoreSet, RetrievalError> {
    let representation = RepresentationName::new("graph_context_v1");
    let components = BTreeMap::from([
        ("algorithm".to_string(), "hierarchy_bfs_v1".to_string()),
        ("seed_rank".to_string(), seed_rank.to_string()),
        ("depth".to_string(), depth.to_string()),
        ("confidence_milli".to_string(), confidence_milli.to_string()),
        ("representation".to_string(), representation.0.clone()),
    ]);
    score_set(
        RetrievalScoreKind::Graph,
        i64::from(raw_score),
        raw_rank,
        RetrievalScoreScale::rank_derived("seed_rank_depth_confidence_v1"),
        representation,
        RetrievalScoreFingerprint::new(
            RetrievalModelFingerprint::new("graph:hierarchy-bfs:v1".to_string())?,
            components,
        ),
    )
}

fn score_set(
    score_kind: RetrievalScoreKind,
    raw_score: i64,
    raw_rank: u32,
    scale: RetrievalScoreScale,
    representation: RepresentationName,
    fingerprint: RetrievalScoreFingerprint,
) -> Result<RetrievalScoreSet, RetrievalError> {
    Ok(RetrievalScoreSet::single(RetrievalLaneScore::new(
        score_kind,
        raw_score,
        RetrievalRawRank::ranked(raw_rank),
        scale,
        representation,
        fingerprint,
    ))?)
}

fn descriptor_fingerprint(
    descriptor: &RetrieverDescriptor,
    lane: &str,
    mut components: BTreeMap<String, String>,
) -> Result<RetrievalScoreFingerprint, RetrievalError> {
    components.extend([
        ("retriever_id".to_string(), descriptor.id.clone()),
        ("modality".to_string(), descriptor.modality.clone()),
        (
            "representation".to_string(),
            descriptor.representation.0.clone(),
        ),
        (
            "generation".to_string(),
            descriptor.generation.value().to_string(),
        ),
    ]);
    Ok(RetrievalScoreFingerprint::new(
        RetrievalModelFingerprint::new(format!(
            "retriever:{}:{}:{}:{}",
            lane,
            descriptor.id,
            descriptor.representation.0,
            descriptor.generation.value()
        ))?,
        components,
    ))
}
