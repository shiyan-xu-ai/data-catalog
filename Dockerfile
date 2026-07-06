# Multi-stage build for the catalog-api container (Cloud Run / Apps Platform).
#
# Stage 1 — frontend: build the Vite SPA to dist/.
# Stage 2 — builder:  compile the workspace to the catalog-api binary (glibc).
# Stage 3 — runtime:  debian-slim with the binary, the built SPA, and CA certs.
#
# The one container serves both the REST API and the SPA on a single port (Cloud
# Run injects $PORT); there is no separate web host and no CORS. Builder and
# runtime are both Debian/glibc, so the dynamically-linked binary runs as-is.

# ── Stage 1: frontend ─────────────────────────────────────────────────────────
FROM oven/bun:1.3.14 AS frontend

WORKDIR /web
# Manifest first for layer caching: deps only re-resolve when they change.
COPY frontend/package.json frontend/bun.lock ./
RUN bun install --frozen-lockfile
COPY frontend/ ./
RUN bun run build   # tsc --noEmit && vite build -> /web/dist

# ── Stage 2: builder ──────────────────────────────────────────────────────────
# Bookworm-based so the binary links glibc that matches the bookworm runtime below.
FROM rust:1.94-slim-bookworm AS builder

# protobuf-compiler + libprotobuf-dev: lance-encoding's build script runs protoc,
# and its .proto files import the well-known types (google/protobuf/empty.proto)
# that libprotobuf-dev installs under /usr/include.
# cmake + clang: aws-lc-sys (pulled in by lance-io's AWS SDK) builds its C/asm
# crypto and generates bindings at build time.
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    protobuf-compiler \
    libprotobuf-dev \
    cmake \
    clang \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Workspace manifests first (layer-cache friendly). rust-toolchain.toml pins the
# channel, so rustup provisions the matching toolchain in-image.
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY catalog-core/Cargo.toml ./catalog-core/
COPY catalog-store/Cargo.toml ./catalog-store/
COPY catalog-api/Cargo.toml ./catalog-api/

# Stub sources so cargo can resolve + fetch + build deps without the real code.
RUN mkdir -p catalog-core/src catalog-store/src catalog-api/src && \
    echo "pub fn _stub() {}" > catalog-core/src/lib.rs && \
    echo "pub fn _stub() {}" > catalog-store/src/lib.rs && \
    echo "pub fn _stub() {}" > catalog-api/src/lib.rs && \
    echo "fn main() {}" > catalog-api/src/main.rs

RUN cargo fetch

# Warm the dependency cache from the stubs. `|| true` tolerates the stub bin;
# stderr is preserved so a real dependency compile error still shows in the log.
RUN cargo build --release --bin catalog-api || true

# Real source, then the final build.
COPY catalog-core/src ./catalog-core/src
COPY catalog-store/src ./catalog-store/src
COPY catalog-api/src  ./catalog-api/src
RUN find catalog-core/src catalog-store/src catalog-api/src -name "*.rs" -exec touch {} +
RUN cargo build --release --bin catalog-api

# ── Stage 3: runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# CA certs for TLS to S3, Secret Manager, and the metadata server.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 1000 --no-create-home catalog

COPY --from=builder /build/target/release/catalog-api /usr/local/bin/catalog-api
COPY catalog-config.yaml ./catalog-config.yaml
COPY --from=frontend /web/dist /app/webui

# Point the server at the bundled SPA; Cloud Run overrides PORT at runtime.
ENV CATALOG_WEBUI_DIR=/app/webui

USER catalog

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/catalog-api"]
