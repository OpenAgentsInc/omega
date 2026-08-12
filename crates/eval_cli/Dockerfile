# Build eval-cli for Linux.
#
# Usage (from the zed repo root):
#   docker build --platform linux/amd64 -f crates/eval_cli/Dockerfile -t eval-cli-builder .
#   docker cp "$(docker create eval-cli-builder)":/eval-cli ./target/eval-cli
#
# Or use the helper script:
#   crates/eval_cli/script/build-linux

FROM rust:1.95.0-bookworm@sha256:6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1 AS builder

WORKDIR /app

# Pre-install the toolchain specified in rust-toolchain.toml so it is cached.
RUN rustup toolchain install 1.95.0 --profile minimal \
    --component rustfmt --component clippy --component rust-analyzer --component rust-src \
    --target wasm32-wasip2 --target wasm32-unknown-unknown --target x86_64-unknown-linux-gnu

# Install build tools: a C toolchain for -sys crates and grammars,
# cmake for aws-lc-sys, and OpenSSL headers for native-tls.
# The image is pinned to Debian 12 so the resulting glibc baseline remains
# compatible with supported contemporary Linux hosts.
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    build-essential \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,id=eval-cli-bookworm-gnu-target,target=/app/target \
    cargo build --profile release-fast --package eval_cli \
        --jobs 1 \
        --target x86_64-unknown-linux-gnu && \
    cp /app/target/x86_64-unknown-linux-gnu/release-fast/eval-cli /eval-cli && \
    strip /eval-cli

FROM scratch
COPY --from=builder /eval-cli /eval-cli
