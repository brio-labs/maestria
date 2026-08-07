#!/usr/bin/env bash
set -euo pipefail
export PATH="${HOME}/.cargo/bin:${PATH}"

CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
SYSROOT_DIR="$(rustc --print sysroot 2>/dev/null || echo "")"
if [[ -n "$SYSROOT_DIR" ]]; then
    RUSTUP_HOME_DIR="$(dirname "$(dirname "$SYSROOT_DIR")")"
else
    RUSTUP_HOME_DIR="${RUSTUP_HOME:-$HOME/.rustup}"
fi
WORKSPACE_DIR="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
REMAP_FLAGS=""
# Order matters: rustc lets the LAST matching prefix win, so list remaps from
# the most general ($HOME) to the most specific ($CARGO_HOME). This makes
# e.g. $HOME/.cargo map to /cargo (not /home/user/.cargo) on every machine,
# regardless of where home and cargo actually live.
if [[ -n "$HOME" ]]; then
    REMAP_FLAGS="$REMAP_FLAGS --remap-path-prefix=$HOME=/home/user"
fi
if [[ -n "$WORKSPACE_DIR" ]]; then
    REMAP_FLAGS="$REMAP_FLAGS --remap-path-prefix=$WORKSPACE_DIR=/workspace"
fi
if [[ -n "$RUSTUP_HOME_DIR" ]]; then
    REMAP_FLAGS="$REMAP_FLAGS --remap-path-prefix=$RUSTUP_HOME_DIR=/rustup"
fi
if [[ -n "$SYSROOT_DIR" ]]; then
    REMAP_FLAGS="$REMAP_FLAGS --remap-path-prefix=$SYSROOT_DIR=/sysroot"
fi
REMAP_FLAGS="$REMAP_FLAGS --remap-path-prefix=$CARGO_HOME_DIR=/cargo"

export RUSTFLAGS="${RUSTFLAGS:-} $REMAP_FLAGS"

# Studio bundle mode: fast (quick PR checks) by default; CI passes "release"
# on main branch and release tags for the optimized bundle.
export STUDIO_BUNDLE_MODE="${STUDIO_BUNDLE_MODE:-fast}"

# Studio's committed Dioxus bundle is built after installing the pinned
# Tailwind and Playwright toolchain, then checked for deterministic drift.
corepack pnpm@9.15.9 --dir web install --frozen-lockfile
RUSTC_WRAPPER="" corepack pnpm@9.15.9 --dir web build
bash scripts/verify-studio-assets.sh
cargo test -p maestria-studio-web
cargo check --target wasm32-unknown-unknown -p maestria-studio-web
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
