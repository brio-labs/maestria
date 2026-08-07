# Pre-baked CI toolchain image for the Maestria Studio Dioxus workspace.
#
# Contents (all pinned to match the rest of CI):
#   - Ubuntu 24.04 (same base as the hosted runner image)
#   - Node 24.11.1 with corepack (pnpm 9.15.9 pre-seeded, offline)
#   - rustup nightly-2026-06-15 (rustfmt, clippy, rustc-codegen-cranelift,
#     wasm32-unknown-unknown) at RUSTUP_HOME=/usr/local/rustup
#   - Dioxus CLI 0.7.10 (SHA-256 verified, same pin as ci.yml)
#   - dx's pinned build tools pre-seeded at DX_HOME=/opt/dx-data/tools
#     (esbuild 0.27.3, wasm-bindgen 0.2.126, binaryen; dx 0.7.10 resolves the
#     binaryen-127 archive but installs it as binaryen-129 - keep whatever dx
#     downloads so image and local builds run the same tool versions)
#   - Playwright Chromium for @playwright/test 1.54.1 at /ms-playwright
#   - clang + mold (host linker per .cargo/config.toml), build essentials
#
# Determinism gate: the last layer regenerates the committed web/dist (fast
# bundle mode) inside this image and fails the build if the output is not
# byte-identical to the committed bundle. It also pre-seeds the cargo registry
# for the whole workspace (cargo fetch --locked) so CI jobs boot without
# downloads. Keep tool versions aligned with what the rest of CI pins.

FROM ubuntu:24.04

SHELL ["/bin/bash", "-eo", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    PATH=/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    DX_HOME=/opt/dx-data \
    COREPACK_HOME=/usr/local/share/corepack \
    PLAYWRIGHT_BROWSERS_PATH=/ms-playwright

RUN apt-get update && apt-get install -y --no-install-recommends \
        git curl ca-certificates xz-utils python3 build-essential clang mold \
    && git config --system --add safe.directory '*' \
    && rm -rf /var/lib/apt/lists/*

# Node 24.11.1 (matches the CI pin) with pnpm 9.15.9 pre-seeded in corepack.
RUN curl -fsSL https://nodejs.org/dist/v24.11.1/node-v24.11.1-linux-x64.tar.xz \
        | tar -xJ -C /usr/local --strip-components=1 \
    && node --version \
    && corepack prepare pnpm@9.15.9 --activate

# Rust toolchain pinned by rust-toolchain.toml (nightly-2026-06-15).
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal \
        --default-toolchain nightly-2026-06-15 \
    && rustup component add rustfmt clippy rustc-codegen-cranelift \
    && rustup target add wasm32-unknown-unknown \
    && rustc --version

# Pinned Dioxus CLI 0.7.10 (SHA-256 verified, same pin as ci.yml).
RUN set -euo pipefail; \
    tmp="$(mktemp -d)"; \
    curl -fsSL --retry 3 \
        https://github.com/DioxusLabs/dioxus/releases/download/v0.7.10/dx-x86_64-unknown-linux-gnu.tar.gz \
        -o "$tmp/dx.tar.gz"; \
    printf '%s  %s\n' \
        '4363e4ed2a3f1eb7f4d38d2d59aed59ce43271c44c16b425e92c89a64761fbe7' \
        "$tmp/dx.tar.gz" | sha256sum -c -; \
    tar -xzf "$tmp/dx.tar.gz" -C "$tmp"; \
    install -m 0755 "$tmp/dx" /usr/local/cargo/bin/dx; \
    rm -rf "$tmp"; \
    dx --version

# Playwright Chromium matching web's @playwright/test 1.54.1.
RUN corepack pnpm@9.15.9 dlx playwright@1.54.1 install --with-deps chromium

# Determinism gate + tool/registry pre-seed (see header comment).
COPY . /src
RUN cd /src \
    && cargo fetch --locked \
    && cd web \
    && corepack pnpm@9.15.9 install --frozen-lockfile \
    && cp -r dist /tmp/committed-dist \
    && STUDIO_BUNDLE_MODE=fast bash scripts/build-studio.sh \
    && echo '=== tool versions ===' \
    && rustc --version \
    && cargo --version \
    && dx --version \
    && /opt/dx-data/tools/esbuild-0.27.3/esbuild --version \
    && /opt/dx-data/tools/wasm-bindgen-0.2.126/wasm-bindgen --version \
    && /opt/dx-data/tools/binaryen-129/bin/wasm-opt --version \
    && echo '=== captured args ===' \
    && (ls /src/target/dx/.captured-args/ 2>/dev/null || true) \
    && (grep -o '"--remap-path-prefix=[^"]*"' /src/target/dx/.captured-args/*/maestria_studio_web.bin.json 2>/dev/null | sort -u || true) \
    && (grep -o 'metadata=[a-f0-9]*\|extra-filename=[-a-f0-9]*\|--cfg [a-z_0-9]*="[^"]*"' /src/target/dx/.captured-args/*/maestria_studio_web.bin.json 2>/dev/null | sort -u || true) \
    && echo '=== exe hashes ===' \
    && (sha256sum /src/target/wasm32-unknown-unknown/wasm-release/deps/maestria_studio_web-*.wasm 2>/dev/null || true) \
    && echo '=== bindgen output hashes ===' \
    && (sha256sum /src/target/dx/maestria-studio-web/release/web/public/wasm/* 2>/dev/null || true) \
    && echo '=== generated asset hashes ===' \
    && sha256sum dist/index.html dist/assets/* \
    && echo '=== committed asset hashes ===' \
    && sha256sum /tmp/committed-dist/index.html /tmp/committed-dist/assets/* \
    && echo '=== generated wasm path tokens ===' \
    && (strings dist/assets/*.wasm | grep -E '^/(home|usr|opt|src|root|rust|workspace|cargo|sysroot)' | sort -u | head -80 || true) \
    && diff -r --brief /tmp/committed-dist dist \
    && rm -rf /src/web/node_modules /src/target /src/web/dist /tmp/committed-dist \
    && ls /opt/dx-data/tools

