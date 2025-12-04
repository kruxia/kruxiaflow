# == DEVELOP ==
FROM rust:1.90-bookworm AS develop

# Install dependencies for profiling
RUN apt-get update && apt-get install -y \
    # PostgreSQL client for migrations
    postgresql-client \
    # utilities
    curl \
    && rm -rf /var/lib/apt/lists/*

# Set working environment
WORKDIR /opt

# Install sqlx-cli for migrations
RUN cargo install sqlx-cli --no-default-features --features postgres,rustls

COPY ./docker-entrypoint-develop.sh /opt/docker-entrypoint-develop.sh
ENTRYPOINT [ "/opt/docker-entrypoint-develop.sh" ]

# Default command - arguments passed to "streamflow serve"
# The entrypoint script will always run: target/profiling/streamflow serve "$@"
EXPOSE 8080

# == BUILD ==
FROM develop AS build
ARG SQLX_OFFLINE=true
COPY ./ ./
RUN cargo build --release

# == DEPLOY ==
# Minimal production image - distroless with single binary
# Migrations are embedded at compile time, no shell or external tools needed
# Uses cc variant for glibc compatibility (static-debian12 for pure static builds)
FROM gcr.io/distroless/cc-debian12:nonroot AS deploy

# Copy single binary (migrations embedded at compile time)
COPY --from=build /opt/target/release/streamflow /streamflow

EXPOSE 8080

# Direct binary execution - no shell needed
# --migrate: wait for postgres, run migrations
# --seed-client: seed OAuth client (idempotent - skip if exists)
ENTRYPOINT ["/streamflow"]
CMD ["serve", "--migrate", "--seed-client"]

# == PROFILING ==
# Profiling environment for StreamFlow
# This provides a Linux environment where jemalloc profiling works correctly
FROM rust:1.90-bookworm AS profiling

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

# Install sqlx-cli for migrations
RUN cargo install sqlx-cli --no-default-features --features postgres,rustls

# Set working directory
WORKDIR /opt

COPY docker-entrypoint-profiling.sh docker-entrypoint-profiling.sh
ENTRYPOINT [ "/opt/docker-entrypoint-profiling.sh" ]

# Default command - arguments passed to "streamflow serve"
# The entrypoint script will always run: target/profiling/streamflow serve "$@"
EXPOSE 8080
