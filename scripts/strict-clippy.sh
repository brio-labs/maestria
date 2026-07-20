#!/usr/bin/env bash
set -euo pipefail

# Nightly adds map_or_identity; map_or is intentional because unwrap_or is disallowed.
cargo clippy --workspace --all-targets --all-features -- \
  -D warnings \
  -D clippy::too_many_lines \
  -D clippy::cognitive_complexity \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::panic \
  -D clippy::disallowed_methods \
  -A clippy::map_or_identity
