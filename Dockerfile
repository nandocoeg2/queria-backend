# Stage 1: Build
FROM rust:1.85-slim-bookworm AS builder
WORKDIR /usr/src/queria
# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev cmake g++ build-essential && rm -rf /var/lib/apt/lists/*
# Copy workspace files
COPY . .
# Build all release binaries
RUN cargo build --release --workspace

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
