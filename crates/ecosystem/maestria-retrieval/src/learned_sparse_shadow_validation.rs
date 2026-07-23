use maestria_domain::{
    ProjectionNamespace, RetrievalLaneScore, RetrievalScoreKind, RetrievalScoreSet,
};

use super::{
    LearnedSparseShadowLaneStatus, LearnedSparseShadowObservation, LearnedSparseShadowStoreError,
    MAX_SHADOW_CANDIDATES_PER_LANE, MAX_SHADOW_CONTRIBUTIONS, MAX_SHADOW_ERROR_CHARS,
    MAX_SHADOW_RETRIEVERS, SHADOW_SCHEMA_VERSION,
};

pub(super) fn bind_namespace(
    bound: &mut Option<ProjectionNamespace>,
    incoming: Option<&ProjectionNamespace>,
) -> Result<(), LearnedSparseShadowStoreError> {
    let Some(incoming) = incoming else {
        return Ok(());
    };
    if let Some(expected) = bound.as_ref()
        && expected != incoming
    {
        return Err(LearnedSparseShadowStoreError::NamespaceMismatch {
            expected: expected.clone(),
            actual: incoming.clone(),
        });
    }
    if bound.is_none() {
        *bound = Some(incoming.clone());
    }
    Ok(())
}

pub(super) fn validate_observations(
    observations: &[LearnedSparseShadowObservation],
) -> Result<Option<ProjectionNamespace>, LearnedSparseShadowStoreError> {
    let mut namespace = None;
    for observation in observations {
        validate_observation(observation)?;
        bind_namespace(&mut namespace, Some(&observation.namespace))?;
    }
    Ok(namespace)
}

pub(super) fn validate_observation(
    observation: &LearnedSparseShadowObservation,
) -> Result<(), LearnedSparseShadowStoreError> {
    if observation.schema_version != SHADOW_SCHEMA_VERSION {
        return Err(LearnedSparseShadowStoreError::InvalidObservation(
            "unsupported schema version".to_string(),
        ));
    }
    observation.namespace.validate().map_err(|error| {
        LearnedSparseShadowStoreError::InvalidObservation(error.to_string())
    })?;
    if observation.lanes.is_empty() || observation.lanes.len() > MAX_SHADOW_RETRIEVERS {
        return Err(LearnedSparseShadowStoreError::InvalidObservation(
            "retriever lane count is outside its bounded range".to_string(),
        ));
    }
    for lane in &observation.lanes {
        if lane.namespace != observation.namespace {
            return Err(LearnedSparseShadowStoreError::NamespaceMismatch {
                expected: observation.namespace.clone(),
                actual: lane.namespace.clone(),
            });
        }
        if lane.retriever_id.trim().is_empty()
            || lane.candidates.len() > MAX_SHADOW_CANDIDATES_PER_LANE
            || lane.candidates.iter().any(|candidate| {
                candidate.reason.contributions.len() > MAX_SHADOW_CONTRIBUTIONS
                    || candidate.score.score_kind != RetrievalScoreKind::LearnedSparse
                    || RetrievalScoreSet::single(candidate.score.clone()).is_err()
                    || !score_namespace_matches(&candidate.score, &observation.namespace)
            })
        {
            return Err(LearnedSparseShadowStoreError::InvalidObservation(
                "lane identity or bounded candidate provenance is invalid".to_string(),
            ));
        }
        if let LearnedSparseShadowLaneStatus::Failed { error } = &lane.status
            && error.chars().count() > MAX_SHADOW_ERROR_CHARS
        {
            return Err(LearnedSparseShadowStoreError::InvalidObservation(
                "failure reason exceeds the bounded error cap".to_string(),
            ));
        }
    }
    Ok(())
}

fn score_namespace_matches(score: &RetrievalLaneScore, namespace: &ProjectionNamespace) -> bool {
    let components = &score.fingerprint.components;
    let trust_zone = format!("{:?}", namespace.trust_zone);
    components.get("instance_id") == Some(&namespace.instance_id)
        && components.get("trust_zone") == Some(&trust_zone)
        && components.get("collection_id") == Some(&namespace.collection_id)
}
