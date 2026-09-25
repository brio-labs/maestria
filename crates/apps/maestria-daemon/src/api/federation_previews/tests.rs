use super::super::protocol::{
    CoverageResponse, SearchEvidenceResponse, SearchPathResultResponse, SearchResponse,
};
use super::*;

fn preview_location() -> EvidenceSourceResponse {
    EvidenceSourceResponse::File {
        path: "notes/current.md".to_owned(),
        start_line: 1,
        end_line: 1,
        content_hash: format!("sha256:{}", "a".repeat(64)),
    }
}

#[test]
fn preview_truncation_stops_at_utf8_boundary_and_reports_it() -> anyhow::Result<()> {
    let location = preview_location();

    let truncated = bounded_search_preview("café", location.clone(), 4, usize::MAX)
        .ok_or_else(|| anyhow::anyhow!("UTF-8 bounded preview was omitted"))?;
    assert_eq!(truncated.excerpt, "caf");
    assert!(truncated.truncated);
    assert!(truncated.excerpt.len() <= 4);

    let complete = bounded_search_preview("café", location, 5, usize::MAX)
        .ok_or_else(|| anyhow::anyhow!("complete preview was omitted"))?;
    assert_eq!(complete.excerpt, "café");
    assert!(!complete.truncated);
    Ok(())
}

#[test]
fn one_hundred_previews_fit_the_aggregate_response_budget() -> anyhow::Result<()> {
    const CANDIDATE_COUNT: usize = 100;
    let mut response = FederationSearchResponse {
        provider_realm: maestria_test_support::realm_id(10)
            .map_err(|error| anyhow::anyhow!("{error}"))?,
        graph_degraded: true,
        search: SearchResponse {
            query: "needle".to_owned(),
            query_id: 1,
            trace_id: 2,
            status: "complete".to_owned(),
            fingerprint: "fingerprint".to_owned(),
            index_generation: 3,
            evidence: (0..CANDIDATE_COUNT)
                .map(|evidence_id| SearchEvidenceResponse {
                    evidence_id: evidence_id as u64,
                    artifact_version: 4,
                    source: "notes/current.md:1-1".to_owned(),
                    range_start: 0,
                    range_end: 20,
                    score_schema_version: 1,
                    scores: Vec::new(),
                    trust: "verified".to_owned(),
                    freshness: "up_to_date".to_owned(),
                    preview: None,
                })
                .collect(),
            path_results: Vec::new(),
            coverage: CoverageResponse {
                percent_covered: 100,
                gaps: Vec::new(),
                distinct_sources: 1,
                distinct_documents: 1,
                distinct_sections: 1,
            },
            conflict_count: 0,
        },
    };
    let max_response_bytes =
        super::super::MAX_REQUEST_BYTES.saturating_sub(RESPONSE_EVIDENCE_RESERVE_BYTES + 1);
    let mut response_bytes = serde_json::to_vec(&response)?.len();
    let excerpt = "café ".repeat(256);
    let mut previews = 0;

    for index in 0..CANDIDATE_COUNT {
        let remaining = CANDIDATE_COUNT - index;
        let candidate_budget = max_response_bytes.saturating_sub(response_bytes) / remaining;
        let Some(preview) =
            bounded_search_preview(&excerpt, preview_location(), 1024, candidate_budget)
        else {
            continue;
        };
        let increment = search_preview_increment(&preview);
        response.search.evidence[index].preview = Some(preview);
        response_bytes += increment;
        previews += 1;
        assert_eq!(serde_json::to_vec(&response)?.len(), response_bytes);
        assert!(response_bytes <= max_response_bytes);
    }

    assert!(previews > 0);
    assert!(response_bytes <= max_response_bytes);
    Ok(())
}

#[test]
fn path_results_fit_the_daemon_response_byte_budget() -> anyhow::Result<()> {
    const PATH_RESULT_COUNT: usize = 100;
    let mut response = FederationSearchResponse {
        provider_realm: maestria_test_support::realm_id(10)
            .map_err(|error| anyhow::anyhow!("{error}"))?,
        graph_degraded: true,
        search: SearchResponse {
            query: "needle".to_owned(),
            query_id: 1,
            trace_id: 2,
            status: "complete".to_owned(),
            fingerprint: "fingerprint".to_owned(),
            index_generation: 3,
            evidence: Vec::new(),
            path_results: (0..PATH_RESULT_COUNT)
                .map(|index| SearchPathResultResponse {
                    path: format!("/{index}-{}", "x".repeat(4090)),
                })
                .collect(),
            coverage: CoverageResponse {
                percent_covered: 100,
                gaps: Vec::new(),
                distinct_sources: 0,
                distinct_documents: 0,
                distinct_sections: 0,
            },
            conflict_count: 0,
        },
    };

    fit_search_path_results(&mut response);

    let response_bytes = serde_json::to_vec(&response)?;
    let max_response_bytes =
        super::super::MAX_REQUEST_BYTES.saturating_sub(RESPONSE_EVIDENCE_RESERVE_BYTES + 1);
    assert!(response_bytes.len() <= max_response_bytes);
    assert!(!response.search.path_results.is_empty());
    assert!(response.search.path_results.len() < PATH_RESULT_COUNT);
    assert!(
        response
            .search
            .path_results
            .first()
            .is_some_and(|result| result.path.starts_with("/0-"))
    );
    Ok(())
}

#[test]
fn legacy_search_evidence_without_preview_still_deserializes() -> anyhow::Result<()> {
    let evidence: SearchEvidenceResponse = serde_json::from_str(
        r#"{"evidence_id":1,"artifact_version":2,"source":"notes/current.md:1-1","range_start":0,"range_end":10,"score_schema_version":1,"scores":[],"trust":"verified","freshness":"up_to_date"}"#,
    )?;
    assert!(evidence.preview.is_none());
    let serialized = serde_json::to_value(evidence)?;
    let object = serialized
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("search evidence was not a JSON object"))?;
    assert!(!object.contains_key("preview"));
    Ok(())
}
