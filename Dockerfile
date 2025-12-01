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

# == INIT ==
# Init container for migrations, seeding, and secret generation
# Used by docker-compose to prepare the environment before starting streamflow
FROM debian:bookworm-slim AS init

RUN apt-get update && apt-get install -y --no-install-recommends \
    openssl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt

# Copy migration and seeding tools
COPY --from=build /usr/local/cargo/bin/sqlx /usr/local/bin/sqlx
COPY --from=build /opt/target/release/seed-oauth-client /usr/local/bin/seed-oauth-client
COPY migrations /opt/migrations

# == DEPLOY ==
# Minimal distroless image - no shell, just the binary
FROM gcr.io/distroless/cc-debian12 AS deploy

# Copy only the streamflow binary
COPY --from=build /opt/target/release/streamflow /streamflow

EXPOSE 8080
ENTRYPOINT ["/streamflow"]
CMD ["serve"]

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
