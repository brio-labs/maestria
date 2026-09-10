use maestria_ports::{
    LATE_INTERACTION_SCORE_DENOMINATOR, LATE_INTERACTION_SCORE_SCALE_V1,
    LateInteractionContribution, LateInteractionScorer, LateInteractionScoringResult,
    MultiVectorQuery, MultiVectorSimilarity, PortError, ProviderCallControl, contribution_order,
};

/// Deterministic query-token MaxSim scorer for Stage A.
///
/// The implementation scans only retained document rows. It never pads a
/// document with synthetic zero vectors, which is important when all real
/// similarities are negative.
#[derive(Debug, Clone)]
pub struct MaxSimLateInteractionScorer {
    max_contributions: usize,
}

impl Default for MaxSimLateInteractionScorer {
    fn default() -> Self {
        Self::new()
    }
}

impl MaxSimLateInteractionScorer {
    pub fn new() -> Self {
        Self {
            max_contributions: maestria_ports::DEFAULT_MAX_CONTRIBUTIONS,
        }
    }

    pub fn with_max_contributions(max_contributions: usize) -> Result<Self, PortError> {
        if max_contributions == 0 || max_contributions > 1024 {
            return Err(PortError::invalid_input(
                "late interaction contribution cap",
                "cap must be between one and 1024",
            ));
        }
        Ok(Self { max_contributions })
    }

    pub fn max_contributions(&self) -> usize {
        self.max_contributions
    }
}

impl LateInteractionScorer for MaxSimLateInteractionScorer {
    fn score(
        &self,
        query: &MultiVectorQuery,
        document: &maestria_ports::MultiVectorDocument,
        control: &dyn ProviderCallControl,
    ) -> Result<LateInteractionScoringResult, PortError> {
        validate_score_inputs(query, document)?;
        let mut all = Vec::with_capacity(query.tokens().len());
        let mut aggregate = 0_i64;
        for query_token in query.tokens() {
            let contribution = score_query_token(query, document, query_token, control)?;
            aggregate = aggregate
                .checked_add(contribution.similarity_micros)
                .ok_or_else(|| {
                    PortError::invalid_input(
                        "late interaction score",
                        "fixed-point aggregate overflow",
                    )
                })?;
            all.push(contribution);
        }
        all.sort_by(contribution_order);
        let retained = all.len().min(self.max_contributions);
        let omitted_sum = all[retained..]
            .iter()
            .try_fold(0_i64, |sum, contribution| {
                sum.checked_add(contribution.similarity_micros)
                    .ok_or_else(|| {
                        PortError::invalid_input(
                            "late interaction score",
                            "omitted contribution overflow",
                        )
                    })
            })?;
        Ok(LateInteractionScoringResult {
            aggregate_micros: aggregate,
            denominator: LATE_INTERACTION_SCORE_DENOMINATOR,
            scale: LATE_INTERACTION_SCORE_SCALE_V1,
            total_query_tokens: u32::try_from(all.len()).map_err(|_| {
                PortError::invalid_input("late interaction score", "query token count overflow")
            })?,
            retained_contribution_count: retained as u32,
            omitted_contribution_count: (all.len() - retained) as u32,
            omitted_contribution_sum_micros: omitted_sum,
            contributions: all.into_iter().take(retained).collect(),
        })
    }
}

fn validate_score_inputs(
    query: &MultiVectorQuery,
    document: &maestria_ports::MultiVectorDocument,
) -> Result<(), PortError> {
    if query.identity() != document.identity() {
        return Err(PortError::invalid_input(
            "late interaction identity",
            "query and document identities differ",
        ));
    }
    if query.identity().fingerprint.aggregation
        != maestria_domain::LateInteractionAggregation::QueryTokenMaxThenSum
    {
        return Err(PortError::invalid_input(
            "late interaction aggregation",
            "unsupported aggregation",
        ));
    }
    if query.tokens().is_empty() || document.tokens().is_empty() {
        return Err(PortError::invalid_input(
            "late interaction score",
            "query and document token sets must not be empty",
        ));
    }
    let dimensions = query.identity().fingerprint.base.dimensions as usize;
    if document
        .tokens()
        .iter()
        .any(|token| token.embedding().len() != dimensions)
    {
        return Err(PortError::invalid_input(
            "late interaction score",
            "document dimension does not match query identity",
        ));
    }
    Ok(())
}

fn score_query_token(
    query: &MultiVectorQuery,
    document: &maestria_ports::MultiVectorDocument,
    query_token: &maestria_ports::MultiVectorToken,
    control: &dyn ProviderCallControl,
) -> Result<LateInteractionContribution, PortError> {
    if control.is_cancelled() {
        return Err(PortError::internal(
            "late interaction score",
            "scoring was cancelled",
        ));
    }
    if control.remaining_ms() == 0 {
        return Err(PortError::downstream(
            "late interaction score",
            "scoring deadline elapsed",
        ));
    }
    let mut best: Option<(f64, u32)> = None;
    for document_token in document.tokens() {
        let mut dot = 0.0_f64;
        for (left, right) in query_token
            .embedding()
            .iter()
            .zip(document_token.embedding())
        {
            dot += f64::from(*left) * f64::from(*right);
        }
        if !dot.is_finite() {
            return Err(PortError::invalid_input(
                "late interaction score",
                "similarity is non-finite",
            ));
        }
        let similarity = match query.identity().fingerprint.similarity {
            MultiVectorSimilarity::DotProduct => dot,
            MultiVectorSimilarity::Cosine => {
                if dot > 1.0 + 1e-6 || dot < -1.0 - 1e-6 {
                    return Err(PortError::invalid_input(
                        "late interaction cosine score",
                        "similarity exceeds the unit range",
                    ));
                }
                dot.clamp(-1.0, 1.0)
            }
        };
        if best.is_none_or(|(best_similarity, best_position)| {
            similarity > best_similarity
                || (similarity == best_similarity && document_token.position() < best_position)
        }) {
            best = Some((similarity, document_token.position()));
        }
    }
    let (similarity, document_position) = best.ok_or_else(|| {
        PortError::invalid_input("late interaction score", "document has no retained rows")
    })?;
    Ok(LateInteractionContribution {
        query_position: query_token.position(),
        document_position,
        similarity_micros: fixed_micros(similarity)?,
    })
}

fn fixed_micros(value: f64) -> Result<i64, PortError> {
    let scaled = value * LATE_INTERACTION_SCORE_DENOMINATOR as f64;
    if !scaled.is_finite() || scaled < i64::MIN as f64 || scaled > i64::MAX as f64 {
        return Err(PortError::invalid_input(
            "late interaction fixed-point score",
            "similarity cannot be represented",
        ));
    }
    Ok(scaled.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{
        ContentHash, CorpusSnapshotId, IndexFingerprint, IndexGenerationId,
        LateInteractionAggregation, RealmId, RepresentationName, TrustZone,
    };
    use maestria_ports::{
        MultiVectorCompression, MultiVectorDocument, MultiVectorFingerprint,
        MultiVectorSourceIdentity, MultiVectorToken, MultiVectorTokenPolicy,
    };

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

    fn identity() -> Result<maestria_ports::MultiVectorIdentity, Box<dyn std::error::Error>> {
        let hash = ContentHash::new(format!("sha256:{}", "a".repeat(64)))?;
        let realm = RealmId::try_from("b".repeat(64))?;
        let policy = |prefix_id: u32, max_vectors| MultiVectorTokenPolicy {
            prefix_id: prefix_id.into(),
            prefix_text: "prefix".into(),
            native_token_limit: 8192,
            max_vectors,
            max_utf8_bytes: 8192,
            truncate_right_after_prefix: true,
            retain_special_tokens: true,
            drop_masked_positions: true,
            pad_token_id: 4.into(),
            query_expansion: false,
            lowercase: false,
            punctuation_pruning: false,
        };
        Ok(maestria_ports::MultiVectorIdentity {
            representation: RepresentationName::new("multivector_text_v1"),
            fingerprint: MultiVectorFingerprint {
                base: IndexFingerprint {
                    provider: "test".into(),
                    model: "test".into(),
                    revision: "test".into(),
                    artifact_hash: hash.clone(),
                    dimensions: 2,
                    quantization: "none".into(),
                    query_template_hash: hash.clone(),
                    document_template_hash: hash.clone(),
                    preprocessing_version: "test".into(),
                },
                tokenizer_hash: hash.clone(),
                vocabulary_hash: hash,
                vocabulary_size: 4,
                similarity: MultiVectorSimilarity::Cosine,
                aggregation: LateInteractionAggregation::QueryTokenMaxThenSum,
                normalization_version: "L2PerTokenV1".into(),
                representation_compression: MultiVectorCompression::None,
                query_policy: policy(1, 128),
                document_policy: policy(2, 512),
            },
            generation_id: IndexGenerationId::new(1),
            corpus_snapshot: CorpusSnapshotId::new(1),
            realm,
            trust_zone: TrustZone::Verified,
        })
    }

    fn hash(letter: char) -> Result<ContentHash, Box<dyn std::error::Error>> {
        Ok(ContentHash::new(format!(
            "sha256:{}",
            letter.to_string().repeat(64)
        ))?)
    }

    fn document(
        identity: maestria_ports::MultiVectorIdentity,
        vectors: Vec<Vec<f32>>,
    ) -> Result<MultiVectorDocument, Box<dyn std::error::Error>> {
        let source = MultiVectorSourceIdentity {
            evidence_id: maestria_domain::EvidenceId::new(1),
            artifact_id: maestria_domain::ArtifactId::new(1),
            version_id: maestria_domain::ArtifactVersionId::new(1),
            source_snapshot_hash: hash('c')?,
            span: maestria_domain::EvidenceSpan::new(
                None,
                maestria_domain::SourceLocation::File {
                    path: "x".into(),
                    start_line: 1,
                    end_line: 1,
                },
                maestria_domain::ContentRange::new(0, 1)?,
            )?,
            source_representation_hash: hash('d')?,
        };
        let tokens = vectors
            .into_iter()
            .enumerate()
            .map(|(position, vector)| {
                MultiVectorToken::new(position as u32, vector)
                    .map_err(|error| -> Box<dyn std::error::Error> { Box::new(error) })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(MultiVectorDocument::new(
            identity,
            source,
            tokens.len() as u32,
            false,
            tokens,
        )?)
    }

    #[test]
    fn scores_actual_rows_and_preserves_negative_maxima() -> Result<(), Box<dyn std::error::Error>>
    {
        let identity = identity()?;
        let query = maestria_ports::MultiVectorQuery::new(
            identity.clone(),
            hash('e')?,
            2,
            false,
            vec![
                MultiVectorToken::new(0, vec![1.0, 0.0])?,
                MultiVectorToken::new(1, vec![0.0, 1.0])?,
            ],
        )?;
        let all = document(identity.clone(), vec![vec![1.0, 0.0], vec![0.0, 1.0]])?;
        let one = document(identity, vec![vec![-1.0, 0.0]])?;
        let scorer = MaxSimLateInteractionScorer::new();
        assert_eq!(
            scorer.score(&query, &all, &Control)?.aggregate_micros,
            2_000_000
        );
        assert_eq!(
            scorer.score(&query, &one, &Control)?.aggregate_micros,
            -1_000_000
        );
        Ok(())
    }
}
