# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.93
ARG NODE_VERSION=24

# ---------------------------------------------------------------------------
# Stage 1: Admin UI build (Node.js)
# ---------------------------------------------------------------------------
FROM node:${NODE_VERSION}-bookworm-slim AS admin-ui-builder
WORKDIR /ui
COPY admin-ui/package.json admin-ui/package-lock.json admin-ui/.npmrc ./
RUN npm ci --ignore-scripts
COPY admin-ui/ .
RUN npm run build

# ---------------------------------------------------------------------------
# Stage 2: Rust dependency cache (cargo-chef)
# ---------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-bookworm AS chef
RUN cargo install cargo-chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---------------------------------------------------------------------------
# Stage 3: Rust build
# ---------------------------------------------------------------------------
FROM chef AS builder
ARG BUILD_DATE
ARG VCS_REF
ARG TARGETARCH

# Install fast linker (mold on amd64, lld on arm64)
RUN apt-get update && apt-get install -y --no-install-recommends \
        clang \
        mold \
        lld \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Configure fast linker per arch:
#   amd64 → mold (10x faster than ld)
#   arm64 → lld  (mold's aarch64 support is less mature)
RUN if [ "$TARGETARCH" = "arm64" ] || [ "$TARGETARCH" = "aarch64" ]; then \
        echo '[target.aarch64-unknown-linux-gnu]\nrustflags = ["-C", "link-arg=-fuse-ld=lld"]' > /app/.cargo/config.toml; \
    else \
        echo '[target.x86_64-unknown-linux-gnu]\nlinker = "clang"\nrustflags = ["-C", "link-arg=-fuse-ld=mold"]' > /app/.cargo/config.toml; \
    fi

# Disable incremental compilation (useless in CI, adds overhead)
ENV CARGO_INCREMENTAL=0

# --- Dependency compilation (CACHED LAYER) --------------------------------
# CRITICAL: NO --mount=type=cache on target here. The compiled dependencies
# must land in the Docker IMAGE LAYER so type=registry/type=gha cache captures
# them. Using --mount=type=cache sends them to an ephemeral BuildKit volume
# that is destroyed after each CI run, causing every build to recompile all
# ~900 crates + duckdb-sys from scratch (the 40-minute cold build bug).
#
# The --mount=type=cache for cargo/registry and cargo/git is fine — those are
# download caches (fetched crate sources), not compilation output.
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# --- Application build (only your crates; deps from cook layer) ------------
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --release --package aeterna \
    && cp /app/target/release/aeterna /app/aeterna-bin

# ---------------------------------------------------------------------------
# Stage 4: Runtime
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

ARG BUILD_DATE
ARG VCS_REF

LABEL org.opencontainers.image.title="Aeterna"
LABEL org.opencontainers.image.description="Universal Memory & Knowledge Framework for Enterprise AI Agent Systems"
LABEL org.opencontainers.image.source="https://github.com/aeterna-universe/aeterna"
LABEL org.opencontainers.image.licenses="Apache-2.0"
LABEL org.opencontainers.image.created="${BUILD_DATE}"
LABEL org.opencontainers.image.revision="${VCS_REF}"

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -u 1000 -s /bin/bash aeterna

WORKDIR /app

COPY --from=builder /app/aeterna-bin /usr/local/bin/aeterna
COPY --from=admin-ui-builder /ui/dist /app/admin-ui/dist

RUN chown -R aeterna:aeterna /app

USER aeterna

ENV RUST_LOG=info
ENV AETERNA_CONFIG_PATH=/app/config
ENV AETERNA_ADMIN_UI_PATH=/app/admin-ui/dist

EXPOSE 8080
EXPOSE 9090

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["curl", "--fail", "--silent", "http://localhost:8080/health"] || exit 1

ENTRYPOINT ["/usr/local/bin/aeterna"]
CMD ["serve"]
