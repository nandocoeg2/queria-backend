# Multi-binary backend image (GHCR name: backend).
# Workspace: rust-version 1.88, edition 2024. Binaries: queria-api, queria-mcp, queria-worker, queria-cli.
# (rust-s3 → sysinfo 0.37 needs rustc ≥ 1.88; keep Docker image and workspace MSRV aligned.)
# Ports: API 17671, MCP 17672, worker health 17673. Public edge is Caddy on :17674.
#
# Cache notes (CI BuildKit):
# - Prefer native arm64 runners (no QEMU) for linux/arm64.
# - cargo registry + target use BuildKit cache mounts so deps survive source-only commits.
# - layer: copy manifests first, cook dummy build, then real sources (cargo-chef-free).

# ---- build ----
FROM rust:1.88-slim-bookworm AS builder
WORKDIR /usr/src/queria

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    cmake \
    g++ \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Workspace manifests only (stable when only .rs change).
COPY Cargo.toml Cargo.lock ./
COPY crates/queria-core/Cargo.toml crates/queria-core/Cargo.toml
COPY crates/queria-db/Cargo.toml crates/queria-db/Cargo.toml
COPY crates/queria-search/Cargo.toml crates/queria-search/Cargo.toml
COPY crates/queria-api/Cargo.toml crates/queria-api/Cargo.toml
COPY crates/queria-mcp/Cargo.toml crates/queria-mcp/Cargo.toml
COPY crates/queria-worker/Cargo.toml crates/queria-worker/Cargo.toml
COPY crates/queria-ingestion/Cargo.toml crates/queria-ingestion/Cargo.toml
COPY crates/queria-cli/Cargo.toml crates/queria-cli/Cargo.toml
COPY crates/queria-backup/Cargo.toml crates/queria-backup/Cargo.toml

# Dummy sources so `cargo build` resolves the workspace graph and fills target/ for deps.
# Layout must match real crate types (lib-only / bin-only / lib+bin).
RUN set -e; \
    for c in queria-core queria-db queria-search queria-ingestion queria-backup; do \
      mkdir -p "crates/$c/src"; \
      echo 'pub fn _docker_cache_stub() {}' > "crates/$c/src/lib.rs"; \
    done; \
    for c in queria-worker queria-cli; do \
      mkdir -p "crates/$c/src"; \
      echo 'fn main() {}' > "crates/$c/src/main.rs"; \
    done; \
    for c in queria-api queria-mcp; do \
      mkdir -p "crates/$c/src"; \
      echo 'pub fn _docker_cache_stub() {}' > "crates/$c/src/lib.rs"; \
      echo 'fn main() {}' > "crates/$c/src/main.rs"; \
    done

# Prefetch dependency graph into BuildKit cache mounts (not baked as huge layers).
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/usr/src/queria/target,sharing=locked \
    cargo build --release \
      -p queria-api -p queria-mcp -p queria-worker -p queria-cli

# Real sources (invalidates here on code change; deps stay in cache mounts).
COPY . .

# Workspace crates rebuild; third-party crates reuse target/ + registry mounts.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/usr/src/queria/target,sharing=locked \
    cargo build --release \
      -p queria-api -p queria-mcp -p queria-worker -p queria-cli \
    && mkdir -p /out \
    && cp target/release/queria-api \
          target/release/queria-mcp \
          target/release/queria-worker \
          target/release/queria-cli \
          /out/
# ---- runtime ----
FROM debian:bookworm-slim AS runtime
ARG QUERIA_SOURCE_COMMIT=unknown
ENV QUERIA_SOURCE_COMMIT=${QUERIA_SOURCE_COMMIT}

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    postgresql-client \
    && rm -rf /var/lib/apt/lists/*

COPY --from=trufflesecurity/trufflehog:latest /usr/bin/trufflehog /usr/local/bin/trufflehog
COPY config/trufflehog-include-paths.txt /config/trufflehog-include-paths.txt
COPY config/trufflehog-exclude-paths.txt /config/trufflehog-exclude-paths.txt
ENV QUERIA_TRUFFLEHOG_INCLUDE_PATHS_FILE=/config/trufflehog-include-paths.txt \
    QUERIA_TRUFFLEHOG_EXCLUDE_PATHS_FILE=/config/trufflehog-exclude-paths.txt

COPY --from=builder /out/queria-api /usr/local/bin/queria-api
COPY --from=builder /out/queria-mcp /usr/local/bin/queria-mcp
COPY --from=builder /out/queria-worker /usr/local/bin/queria-worker
COPY --from=builder /out/queria-cli /usr/local/bin/queria-cli
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh

USER 10001:10001
EXPOSE 17671 17672 17673
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["queria-api"]
