# Stage 1: Build
# Pin close to workspace rust-version; edition 2024 needs >=1.85. Prefer bookworm for aarch64 OCI hosts.
FROM rust:1.85-slim-bookworm AS builder
WORKDIR /usr/src/queria
# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev cmake g++ build-essential && rm -rf /var/lib/apt/lists/*
# Copy workspace files
COPY . .
# Build runtime binaries only (faster than full workspace tests)
RUN cargo build --release -p queria-api -p queria-mcp -p queria-worker -p queria-cli

# Stage 2: Runtime
FROM debian:bookworm-slim AS runtime
ARG QUERIA_SOURCE_COMMIT=unknown
ENV QUERIA_SOURCE_COMMIT=${QUERIA_SOURCE_COMMIT}
# Install CA certificates, git, and clean apt cache
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    postgresql-client \
    && rm -rf /var/lib/apt/lists/*
# Copy TruffleHog from the official TruffleSecurity image
COPY --from=trufflesecurity/trufflehog:latest /usr/bin/trufflehog /usr/local/bin/trufflehog
# Bake secret-scan path filters so the worker needs no host config bind-mount.
COPY config/trufflehog-include-paths.txt /config/trufflehog-include-paths.txt
COPY config/trufflehog-exclude-paths.txt /config/trufflehog-exclude-paths.txt
ENV QUERIA_TRUFFLEHOG_INCLUDE_PATHS_FILE=/config/trufflehog-include-paths.txt \
    QUERIA_TRUFFLEHOG_EXCLUDE_PATHS_FILE=/config/trufflehog-exclude-paths.txt
# Copy release binaries from the builder
COPY --from=builder /usr/src/queria/target/release/queria-api /usr/local/bin/queria-api
COPY --from=builder /usr/src/queria/target/release/queria-mcp /usr/local/bin/queria-mcp
COPY --from=builder /usr/src/queria/target/release/queria-worker /usr/local/bin/queria-worker
COPY --from=builder /usr/src/queria/target/release/queria-cli /usr/local/bin/queria-cli

# Copy entrypoint script
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh

# Run as non-root UID
USER 10001:10001

# Expose ports (edge/Caddy is a separate container on 17674)
EXPOSE 17671 17672 17673

# Entrypoint
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["queria-api"]
