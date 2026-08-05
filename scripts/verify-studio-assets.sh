#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
WEB_DIR="$ROOT_DIR/web"
EXPECTED_ASSETS=$'web/dist/app.css\nweb/dist/app.js\nweb/dist/index.html'
ACTUAL_ASSETS="$(git -C "$ROOT_DIR" ls-files web/dist)"
if [[ "$ACTUAL_ASSETS" != "$EXPECTED_ASSETS" ]]; then
    echo "::error::Studio dist must contain exactly the committed Vite asset set." >&2
    printf '%s\n' "$ACTUAL_ASSETS" >&2
    exit 1
fi

for asset in index.html app.js app.css; do
    if [[ ! -s "$WEB_DIR/dist/$asset" ]]; then
        echo "::error::Studio Vite asset missing or empty: $asset" >&2
        exit 1
    fi
done

if ! git -C "$ROOT_DIR" diff --quiet -- web/dist; then
    echo "::error::Studio dist differs from the committed deterministic Vite bundle." >&2
    git -C "$ROOT_DIR" diff -- web/dist
    exit 1
fi

if [[ -n "$(git -C "$ROOT_DIR" status --short --untracked-files=all -- web/dist)" ]]; then
    echo "::error::Studio dist contains untracked files; commit the deterministic Vite bundle." >&2
    exit 1
fi

printf '%s\n' 'Studio Vite assets match the committed deterministic bundle.'
