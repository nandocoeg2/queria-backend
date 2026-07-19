# Multi-binary backend image (GHCR name: backend).
# Workspace: rust-version 1.85, edition 2024. Binaries: queria-api, queria-mcp, queria-worker, queria-cli.
# Ports: API 17671, MCP 17672, worker health 17673. Public edge is Caddy on :17674.

# ---- build ----
# Prefer bookworm for aarch64 OCI hosts (production is Oracle arm64).
FROM rust:1.85-slim-bookworm AS builder
WORKDIR /usr/src/queria

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    cmake \
    g++ \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

COPY . .
# Runtime packages only (skip full workspace tests in image build).
RUN cargo build --release -p queria-api -p queria-mcp -p queria-worker -p queria-cli

# ---- runtime ----
FROM debian:bookworm-slim AS runtime
ARG QUERIA_SOURCE_COMMIT=unknown
ENV QUERIA_SOURCE_COMMIT=${QUERIA_SOURCE_COMMIT}

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    postgresql-client \
    && rm -rf /var/lib/apt/lists/*

# Worker secret scan + path filters baked into the image (no host bind-mount required).
COPY --from=trufflesecurity/trufflehog:latest /usr/bin/trufflehog /usr/local/bin/trufflehog
COPY config/trufflehog-include-paths.txt /config/trufflehog-include-paths.txt
COPY config/trufflehog-exclude-paths.txt /config/trufflehog-exclude-paths.txt
ENV QUERIA_TRUFFLEHOG_INCLUDE_PATHS_FILE=/config/trufflehog-include-paths.txt \
    QUERIA_TRUFFLEHOG_EXCLUDE_PATHS_FILE=/config/trufflehog-exclude-paths.txt

COPY --from=builder /usr/src/queria/target/release/queria-api /usr/local/bin/queria-api
COPY --from=builder /usr/src/queria/target/release/queria-mcp /usr/local/bin/queria-mcp
COPY --from=builder /usr/src/queria/target/release/queria-worker /usr/local/bin/queria-worker
COPY --from=builder /usr/src/queria/target/release/queria-cli /usr/local/bin/queria-cli
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh

USER 10001:10001
EXPOSE 17671 17672 17673
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["queria-api"]
