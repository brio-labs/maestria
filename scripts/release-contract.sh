#!/usr/bin/env bash
set -euo pipefail

cargo test -p maestria-cli --test release_contract -- --test-threads=1 --nocapture
cargo test -p maestria-daemon --test vertical_slice -- --nocapture
cargo test -p maestria-runtime runtime_evidence_tests::fetch_web_records_hashed_blob_and_security_boundary -- --nocapture
cargo test -p maestria-runtime runtime_validation_gate_tests -- --nocapture
cargo test -p maestria-retrieval --test adaptive_contract_tests
cargo test -p maestria-retrieval --test contract_tests
cargo test -p maestria-retrieval --test golden_fixture
cargo test -p maestria-core --test golden_gate
