# Multi-stage Rust build for catalog-api.
#
# Stage 1 — builder: compile the workspace and produce the catalog-api binary.
# Stage 2 — runtime: minimal Debian-slim image with only the binary and CA certs.
#
# The binary is statically linked via musl for reproducibility. Override
# RUST_TOOLCHAIN to pin a different toolchain; the default matches rust-toolchain.toml.

# ── Stage 1: builder ────────────────────────────────────────────────────────────
FROM rust:1.87-slim AS builder

# Install musl cross-compilation target and C linker.
# protobuf-compiler: lance-encoding's build script compiles .proto files and
# needs the protoc binary at build time.
RUN apt-get update && apt-get install -y --no-install-recommends \
    musl-tools \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /build

# Copy workspace manifests first (layer-cache friendly: dependency changes invalidate
# fewer layers than source changes).
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY catalog-core/Cargo.toml ./catalog-core/
COPY catalog-store/Cargo.toml ./catalog-store/
COPY catalog-api/Cargo.toml ./catalog-api/

# Stub out each crate with an empty lib/main so cargo can resolve and fetch deps
# without the full source. This layer is cached until a Cargo.toml changes.
RUN mkdir -p catalog-core/src catalog-store/src catalog-api/src && \
    echo "pub fn _stub() {}" > catalog-core/src/lib.rs && \
    echo "pub fn _stub() {}" > catalog-store/src/lib.rs && \
    echo "pub fn _stub() {}" > catalog-api/src/lib.rs && \
    echo "fn main() {}" > catalog-api/src/main.rs

RUN cargo fetch --target x86_64-unknown-linux-musl

# Build deps only (the stubs) to warm the cache. `|| true` tolerates the stub bin failing to
# produce a usable binary; stderr is kept (not sent to /dev/null) so a real dependency compile
# error is visible in the build log instead of surfacing later as a slow failure.
RUN cargo build --release --target x86_64-unknown-linux-musl --bin catalog-api || true

# Now copy real source and do the final build.
COPY catalog-core/src ./catalog-core/src
COPY catalog-store/src ./catalog-store/src
COPY catalog-api/src  ./catalog-api/src

# Touch src files so cargo detects the change after the stub build above.
RUN find catalog-core/src catalog-store/src catalog-api/src -name "*.rs" -exec touch {} +

RUN cargo build --release --target x86_64-unknown-linux-musl --bin catalog-api

# ── Stage 2: runtime ────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# CA certificates for TLS connections to S3 / k8s API.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Non-root user.
RUN useradd --system --uid 1000 --no-create-home catalog

COPY --from=builder \
    /build/target/x86_64-unknown-linux-musl/release/catalog-api \
    /usr/local/bin/catalog-api

USER catalog

# API port (also documented in deploy/base/service.yaml)
EXPOSE 8080
# Internal metrics + healthz port
EXPOSE 9090

ENTRYPOINT ["/usr/local/bin/catalog-api"]
