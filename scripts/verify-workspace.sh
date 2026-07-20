#!/usr/bin/env bash
set -euo pipefail

python3 scripts/version.py check
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
bash scripts/strict-clippy.sh
cargo test --workspace --all-targets --all-features
bash scripts/release-contract.sh
cargo test --workspace --doc --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
cargo deny check all
cargo machete
cargo tree --locked --duplicates
python3 scripts/philosophy-check.py
python3 scripts/codeowners-check.py
python3 scripts/doc-consistency-check.py
python3 -m unittest discover -s scripts -p 'test_*.py'
