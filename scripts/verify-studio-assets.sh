#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
WEB_DIR="$ROOT_DIR/web"
DIST_DIR="$WEB_DIR/dist"
TRACKED_ASSETS="$(git -C "$ROOT_DIR" ls-files 'web/dist/*')"

if [[ -z "$TRACKED_ASSETS" ]]; then
    echo "::error::Studio dist has no committed assets." >&2
    exit 1
fi

require_nonempty() {
    local path="$1"
    if [[ ! -s "$ROOT_DIR/$path" ]]; then
        echo "::error::Studio asset missing or empty: $path" >&2
        echo "::error::Generated Studio assets:" >&2
        find "$DIST_DIR/assets" -maxdepth 1 -type f -printf '  %f\n' | sort >&2
        exit 1
    fi
}

require_nonempty web/dist/index.html

css_asset=""
while IFS= read -r asset; do
    case "$asset" in
        web/dist/tailwind.css|web/dist/assets/tailwind.css)
            css_asset="$asset"
            break
            ;;
    esac
done <<< "$TRACKED_ASSETS"
if [[ -z "$css_asset" ]]; then
    echo "::error::Committed Dioxus dist must contain tailwind.css." >&2
    exit 1
fi
require_nonempty "$css_asset"

js_asset=""
wasm_asset=""
while IFS= read -r asset; do
    case "$asset" in
        *.js) [[ -z "$js_asset" ]] && js_asset="$asset" ;;
        *.wasm) [[ -z "$wasm_asset" ]] && wasm_asset="$asset" ;;
    esac
done <<< "$TRACKED_ASSETS"
[[ -n "$js_asset" ]] || { echo "::error::No committed Dioxus JavaScript loader." >&2; exit 1; }
[[ -n "$wasm_asset" ]] || { echo "::error::No committed Dioxus WASM asset." >&2; exit 1; }
require_nonempty "$js_asset"
require_nonempty "$wasm_asset"

python3 - "$DIST_DIR/index.html" "${js_asset#web/dist/}" <<'PY'
from pathlib import Path
import sys

index_path, loader = sys.argv[1:]
index = Path(index_path).read_text(encoding="utf-8")
if loader not in index:
    raise SystemExit(f"::error::Dioxus index.html does not reference its JavaScript loader: {loader}")
PY

if ! git -C "$ROOT_DIR" diff --quiet -- web/dist; then
    echo "::error::Studio dist differs from the committed deterministic Dioxus bundle." >&2
    git -C "$ROOT_DIR" diff -- web/dist
    exit 1
fi

bundle_status="$(git -C "$ROOT_DIR" status --short --untracked-files=all -- web/dist)"
while IFS= read -r status_line; do
    case "$status_line" in
        "??"*)
            echo "::error::Studio dist contains untracked files; commit the deterministic Dioxus bundle." >&2
            exit 1
            ;;
    esac
done <<< "$bundle_status"

printf '%s\n' 'Studio Dioxus assets match the committed deterministic bundle.'
