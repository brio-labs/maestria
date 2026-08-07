#!/usr/bin/env bash
set -euo pipefail

# Clippy owns semantic failure handling; the philosophy checker owns permanent
# physical readability rules that rustc cannot express as lints.
python3 scripts/philosophy-check.py

# Native crates use the repository-wide disallowed-method policy.
# The runtime's established effect-orchestration handlers are intentionally
# state-machine-shaped and currently exceed the 30-point cognitive budget.
# Keep every other native crate at the strict threshold, and scope the
# exemption to that package while retaining all other failure lints.
cargo clippy --workspace --exclude maestria-studio-web --exclude maestria-runtime --no-deps --all-targets --all-features -- \
  -D warnings \
  -D clippy::too_many_lines \
  -D clippy::cognitive_complexity \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::panic \
  -D clippy::disallowed_methods

cargo clippy -p maestria-runtime --no-deps --all-targets --all-features -- \
  -D warnings \
  -D clippy::too_many_lines \
  -A clippy::cognitive_complexity \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::panic \
  -D clippy::disallowed_methods

# Dioxus expands RSX into generated Option unwraps and HashMap internals at
# every component call site. Source-level failures remain covered by the
# philosophy checker and the frontend's wasm target check.
cargo clippy -p maestria-studio-web --target wasm32-unknown-unknown --all-targets --all-features -- \
  -D warnings \
  -A clippy::disallowed_methods \
  -A clippy::disallowed_types
