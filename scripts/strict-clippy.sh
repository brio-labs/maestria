#!/usr/bin/env bash
set -euo pipefail

# Clippy owns semantic failure handling; the philosophy checker owns permanent
# physical readability rules that rustc cannot express as lints.
python3 scripts/philosophy-check.py

# First-party native crates keep strict semantic lints. Tauri's generated
# Context is handled separately because it expands to internal unwraps and
# HashMaps in the caller, and its fields cannot be constructed externally.
cargo clippy --workspace --exclude maestria-launcher --exclude maestria-studio-web --no-deps --all-targets --all-features -- \
  -D warnings \
  -D clippy::too_many_lines \
  -D clippy::cognitive_complexity \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::panic \
  -D clippy::disallowed_methods

# Keep every other strict lint on the launcher. The philosophy checker rejects
# first-party unwraps, hash collections, and Rust lint-bypass attributes in source.
cargo clippy -p maestria-launcher --no-deps --all-targets --all-features -- \
  -D warnings \
  -D clippy::too_many_lines \
  -D clippy::cognitive_complexity \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::panic \
  -A clippy::disallowed_methods \
  -A clippy::disallowed_types

# Dioxus expands RSX into generated Option unwraps and HashMap internals at
# every component call site. Source-level failures remain covered by the
# philosophy checker and the frontend's wasm target check.
cargo clippy -p maestria-studio-web --target wasm32-unknown-unknown --all-targets --all-features -- \
  -D warnings \
  -A clippy::disallowed_methods \
  -A clippy::disallowed_types
