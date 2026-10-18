use sillage_retrieval::golden::GoldenFixture;

use crate::common::golden::{fixture_gate, multi_query_fixture};

#[test]
fn serialized_multi_query_fixture_passes_the_deterministic_gate()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = multi_query_fixture()?;
    let persisted = include_str!(
        "../../../../../tests/frozen-corpus-snapshots/benchmark-evidence-v1/193a44d4bb2800a3ee19f44728362aa5ccfdc8cc/crates/ecosystem/maestria-retrieval/tests/fixtures/golden-v3.json"
    );
    let decoded: GoldenFixture = serde_json::from_str(persisted)?;
    assert_eq!(decoded, fixture);

    let encoded = serde_json::to_vec(&decoded)?;
    let round_tripped: GoldenFixture = serde_json::from_slice(&encoded)?;
    assert_eq!(round_tripped, decoded);

    let reports = decoded.evaluate(&fixture_gate())?;
    assert_eq!(reports.len(), decoded.corpus.queries.len());
    assert!(reports.iter().all(|report| {
        report.resources.telemetry_complete
            && report.security.telemetry_complete
            && report.resources.latency_ms == 4
            && report.resources.memory_bytes == 100
            && report.resources.disk_bytes == 200
            && report.security.acl_leakage == 0
            && report.security.attack_successes == 0
    }));
    assert!(
        reports
            .iter()
            .any(|report| report.recall_at_k[&10] == sillage_retrieval::golden::Metric::ONE)
    );
    assert!(
        reports
            .iter()
            .any(|report| report.exact_span_recall == sillage_retrieval::golden::Metric::ONE)
    );
    Ok(())
}
