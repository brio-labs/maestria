use maestria_domain::SearchStatus;
use maestria_retrieval::golden::GoldenGateError;

use crate::common::golden::{fixture_gate, multi_query_fixture};

#[test]
fn fixture_mutations_fail_with_typed_gate_errors() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = multi_query_fixture()?;

    let mut trace_mutation = fixture.clone();
    let observation = trace_mutation
        .observations
        .get_mut(1)
        .ok_or("lexical observation missing")?;
    let trace = observation
        .outcome
        .trace_data
        .as_mut()
        .ok_or("lexical trace missing")?;
    trace.original_query.push_str(" mutated");
    assert!(matches!(
        fixture_gate().evaluate_fixture(&trace_mutation),
        Err(GoldenGateError::TraceMismatch { query_id: 102 })
    ));

    let mut route_mutation = fixture.clone();
    let observation = route_mutation
        .observations
        .get_mut(1)
        .ok_or("lexical observation missing")?;
    let trace_id = {
        let trace = observation
            .outcome
            .trace_data
            .as_mut()
            .ok_or("lexical trace missing")?;
        trace.retrievers.push("tampered-route".to_owned());
        trace.deterministic_id()
    };
    observation.outcome.trace = trace_id;
    assert!(matches!(
        fixture_gate().evaluate_fixture(&route_mutation),
        Err(GoldenGateError::TraceMismatch { query_id: 102 })
    ));

    let mut fingerprint_mutation = fixture.clone();
    let observation = fingerprint_mutation
        .observations
        .get_mut(1)
        .ok_or("lexical observation missing")?;
    observation.outcome.fingerprint =
        maestria_domain::RetrievalModelFingerprint::new("trace:mutated".to_string())?;
    assert!(matches!(
        fixture_gate().evaluate_fixture(&fingerprint_mutation),
        Err(GoldenGateError::TraceMismatch { query_id: 102 })
    ));

    let mut status_mutation = fixture.clone();
    let expected = status_mutation
        .corpus
        .queries
        .get_mut(1)
        .ok_or("lexical query missing")?;
    expected.expected_status = SearchStatus::AnswerableWithWarnings;
    assert!(matches!(
        fixture_gate().evaluate_fixture(&status_mutation),
        Err(GoldenGateError::StatusMismatch { query_id: 102, .. })
    ));

    let mut evidence_mutation = fixture;
    let observation = evidence_mutation
        .observations
        .get_mut(1)
        .ok_or("lexical observation missing")?;
    observation.outcome.evidence.clear();
    assert!(matches!(
        fixture_gate().evaluate_fixture(&evidence_mutation),
        Err(GoldenGateError::TraceMismatch { query_id: 102 })
    ));
    Ok(())
}
