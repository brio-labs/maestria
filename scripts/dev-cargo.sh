#!/usr/bin/env bash
set -euo pipefail

if [[ $# -eq 0 ]]; then
  printf 'usage: %s <cargo arguments>\n' "$0" >&2
  exit 2
fi

# Cargo has no profile-scoped rustflags. Keep this unstable compiler flag in
# this development-only wrapper so release builds never inherit it.
if [[ -n "${RUSTFLAGS:-}" ]]; then
  export RUSTFLAGS="${RUSTFLAGS} -Zshare-generics=y"
else
  # Match the repository's Linux linker flags when environment rustflags
  # replace the target-specific flags from .cargo/config.toml.
  export RUSTFLAGS="-C link-arg=-fuse-ld=mold -Zshare-generics=y"
fi

exec cargo "$@"
