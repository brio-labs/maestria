#!/usr/bin/env bash
set -euo pipefail

# Studio's committed bundle is deliberately buildable without Cargo invoking
# Node. Release gates still install the pinned frontend dependencies, exercise
# the frontend checks, rebuild, and reject any dist drift.
corepack pnpm@9.15.9 --dir web install --frozen-lockfile
corepack pnpm@9.15.9 --dir web typecheck
corepack pnpm@9.15.9 --dir web test
corepack pnpm@9.15.9 --dir web test:component
corepack pnpm@9.15.9 --dir web build
bash scripts/verify-studio-assets.sh
cargo test -p maestria-studio -- --nocapture

cargo test -p maestria-cli --test release_contract -- --test-threads=1 --nocapture
cargo test -p maestria-daemon --test vertical_slice -- --nocapture
cargo test -p maestria-runtime runtime_evidence_tests::fetch_web_records_hashed_blob_and_security_boundary -- --nocapture
cargo test -p maestria-runtime runtime_validation_gate_tests -- --nocapture
cargo test -p maestria-retrieval --test adaptive_contract_tests
# Contract fixtures remain policy tests; these suites also execute the frozen
# repository index adapter and the explicit visual-provider degradation path.
cargo test -p maestria-retrieval --test repository_benchmark_tests
cargo test -p maestria-retrieval --test visual_benchmark_tests
cargo test -p maestria-retrieval --test retrieval -- sync_engine --nocapture
cargo test -p maestria-retrieval --test retrieval -- async_engine --nocapture
cargo test -p maestria-retrieval --test retrieval -- golden_fixture
cargo test -p maestria-core --test golden_gate
