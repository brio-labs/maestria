#!/usr/bin/env bash
set -euo pipefail

command -v clang >/dev/null
command -v mold >/dev/null

active_toolchain="$(rustup show active-toolchain)"
case "$active_toolchain" in
  nightly-2026-06-15-*) ;;
  *)
    printf 'expected the pinned nightly toolchain, got: %s\n' "$active_toolchain" >&2
    exit 1
    ;;
esac

host="$(rustc -vV | sed -n 's/^host: //p')"
sysroot="$(rustc --print sysroot)"
backend_pattern="$sysroot/lib/rustlib/$host/codegen-backends/librustc_codegen_cranelift-*.so"
compgen -G "$backend_pattern" >/dev/null

linker_version="$(clang -fuse-ld=mold -Wl,--version 2>&1)"
case "$linker_version" in
  *mold*) ;;
  *)
    printf 'clang did not select mold:\n%s\n' "$linker_version" >&2
    exit 1
    ;;
esac

cargo metadata --no-deps --format-version 1 >/dev/null
printf 'build toolchain verified: %s with Cranelift and mold\n' "$active_toolchain"
