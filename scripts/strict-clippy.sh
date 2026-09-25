#!/usr/bin/env bash
set -euo pipefail

# Clippy owns semantic failure handling; the philosophy checker owns permanent
# physical readability rules that rustc cannot express as lints.
python3 scripts/philosophy-check.py

# Slint expands generated component code into the launcher crate. Clippy sees
# generated unwraps, panics, and accessibility helpers as caller tokens, so
# those lints cannot be scoped away from generated code alone. The philosophy
# checker still rejects handwritten failure methods and panics.
cargo clippy --workspace --exclude maestria-launcher --exclude maestria-studio-web --no-deps --all-targets --all-features -- \
  -D warnings \
  -D clippy::too_many_lines \
  -D clippy::cognitive_complexity \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::panic \
  -D clippy::disallowed_methods

# The philosophy checker rejects forbidden methods, hash collections, and
# lint-bypass attributes in first-party launcher source.
cargo clippy -p maestria-launcher --no-deps --all-targets --all-features -- \
  -D warnings \
  -D clippy::too_many_lines \
  -A clippy::cognitive_complexity \
  -A clippy::unwrap_used \
  -A clippy::expect_used \
  -A clippy::panic \
  -A clippy::disallowed_methods \
  -A clippy::disallowed_types

# Dioxus expands RSX into generated Option unwraps and HashMap internals at
# every component call site. Source-level failures remain covered by the
# philosophy checker and the frontend's wasm target check.
cargo clippy -p maestria-studio-web --target wasm32-unknown-unknown --all-targets --all-features -- \
  -D warnings \
  -A clippy::disallowed_methods \
  -A clippy::disallowed_types
