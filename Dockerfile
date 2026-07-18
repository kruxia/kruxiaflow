# == DEVELOP ==
FROM rust:1.97-bookworm AS develop

# Install dependencies for profiling
RUN apt-get update && apt-get install -y \
    # PostgreSQL client for migrations
    postgresql-client \
    # utilities
    curl \
    && rm -rf /var/lib/apt/lists/*

# Set working environment
WORKDIR /opt

# Install sqlx-cli for migrations — pinned to match the workspace's sqlx
# version; unpinned installs drift (sqlx-cli 0.9.0 requires a newer rustc
# than older base images and can mismatch the workspace's sqlx 0.8 metadata)
RUN cargo install sqlx-cli@0.8.6 --locked --no-default-features --features postgres,rustls

COPY ./docker-entrypoint-develop.sh /opt/docker-entrypoint-develop.sh
ENTRYPOINT [ "/opt/docker-entrypoint-develop.sh" ]

# Default command - arguments passed to "kruxiaflow serve"
# The entrypoint script will always run: target/profiling/kruxiaflow serve "$@"
EXPOSE 8080

# == BUILD ==
FROM develop AS build
ARG SQLX_OFFLINE=true
COPY ./ ./
RUN cargo build --release --features redis-cache

# == DEPLOY ==
# Minimal production image - distroless with single binary
# Migrations are embedded at compile time, no shell or external tools needed
# Uses cc variant for glibc compatibility (static-debian12 for pure static builds)
FROM gcr.io/distroless/cc-debian12:nonroot AS deploy

# Copy single binary (migrations embedded at compile time)
COPY --from=build /opt/target/release/kruxiaflow /kruxiaflow

EXPOSE 8080

# Direct binary execution - no shell needed
# --migrate: wait for postgres, run migrations
# --seed-client: seed OAuth client (idempotent - skip if exists)
# --seed-llm: seed LLM model catalog from YAML file
ENTRYPOINT ["/kruxiaflow"]
CMD ["serve", "--migrate", "--seed-client", "--seed-llm", "/config/llm_models.yaml"]

# == PROFILING ==
# Profiling environment for Kruxia Flow
# This provides a Linux environment where jemalloc profiling works correctly
FROM rust:1.97-bookworm AS profiling

# Install dependencies for profiling
RUN apt-get update && apt-get install -y \
    # jemalloc profiling tools
    libjemalloc-dev \
    google-perftools \
    # graph generation
    graphviz \
    # PostgreSQL client for migrations
    postgresql-client \
    # utilities
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install sqlx-cli for migrations — pinned to match the workspace's sqlx
# version; unpinned installs drift (sqlx-cli 0.9.0 requires a newer rustc
# than older base images and can mismatch the workspace's sqlx 0.8 metadata)
RUN cargo install sqlx-cli@0.8.6 --locked --no-default-features --features postgres,rustls

# Set working directory
WORKDIR /opt

COPY docker-entrypoint-profiling.sh docker-entrypoint-profiling.sh
ENTRYPOINT [ "/opt/docker-entrypoint-profiling.sh" ]

# Default command - arguments passed to "kruxiaflow serve"
# The entrypoint script will always run: target/profiling/kruxiaflow serve "$@"
EXPOSE 8080
