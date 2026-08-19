#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
WEB_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
WORKSPACE_DIR="$(git -C "$WEB_DIR" rev-parse --show-toplevel 2>/dev/null || (cd -- "$WEB_DIR/.." && pwd))"

CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
SYSROOT_DIR="$(rustc --print sysroot 2>/dev/null || echo "")"
if [[ -n "$SYSROOT_DIR" ]]; then
    RUSTUP_HOME_DIR="$(dirname "$(dirname "$SYSROOT_DIR")")"
else
    RUSTUP_HOME_DIR="${RUSTUP_HOME:-$HOME/.rustup}"
fi

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

export RUSTFLAGS="${REMAP_FLAGS# }"

# Studio bundle mode: "fast" skips wasm-opt optimization passes for quick PR
# checks (default); "release" applies the full -Oz pass and is used on main
# branch and release tags. Both modes are byte-for-byte deterministic for a
# given toolchain, which is what verify-studio-assets.sh relies on.
STUDIO_BUNDLE_MODE="${STUDIO_BUNDLE_MODE:-fast}"
case "$STUDIO_BUNDLE_MODE" in
    fast) WASM_OPT_LEVEL="0" ;;
    release) WASM_OPT_LEVEL="z" ;;
    *)
        echo "::error::Unknown STUDIO_BUNDLE_MODE '$STUDIO_BUNDLE_MODE' (expected 'fast' or 'release')." >&2
        exit 1
        ;;
esac

# dx 0.7.10 registers itself as cargo's RUSTC_WORKSPACE_WRAPPER, and cargo bakes
# the wrapper's absolute path into every crate's metadata hash. That makes the
# wasm bytes depend on where dx happens to be installed. Normalize dx to one
# canonical path (the same absolute path on every machine) so builds are
# byte-identical across environments.
DX_BIN="$(command -v dx)"
DX_CANONICAL="/tmp/dx"
if [[ "$DX_BIN" != "$DX_CANONICAL" ]]; then
    install -m 0755 "$DX_BIN" "$DX_CANONICAL"
fi

cd -- "$WEB_DIR"

corepack pnpm@9.15.9 run build:css
rm -rf dist

# dx reads the wasm-opt level from Dioxus.toml. Apply it without touching the
# committed config and restore the file on exit.
config_backup="$(mktemp)"
cp Dioxus.toml "$config_backup"
trap 'mv -f "$config_backup" "$WEB_DIR/Dioxus.toml"' EXIT
python3 - "$WASM_OPT_LEVEL" <<'PY'
import re
import sys

level = sys.argv[1]
path = "Dioxus.toml"
text = open(path, encoding="utf-8").read()
kept = []
skipping = False
for line in text.splitlines():
    stripped = line.strip()
    if skipping:
        if stripped.startswith("["):
            skipping = False
        else:
            continue
    if re.match(r"^\[web\.wasm_opt\]\s*$", stripped):
        skipping = True
        continue
    kept.append(line)
text = "\n".join(kept).rstrip() + "\n"
text += f'\n[web.wasm_opt]\nlevel = "{level}"\n'
open(path, "w", encoding="utf-8").write(text)
PY

"$DX_CANONICAL" bundle --release --debug-symbols false --platform web --out-dir dist

for asset in dist/public/*; do
    mv "$asset" dist/
done
rmdir dist/public
cp assets/tailwind.css dist/assets/tailwind.css
python3 scripts/prune-dioxus-assets.py dist
