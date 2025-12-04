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
# Production image with management tools (sqlx, seed-oauth-client)
# Uses debian-slim for shell access needed by migrations and management
FROM debian:bookworm-slim AS deploy

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt

# Copy binaries and tools
COPY --from=build /opt/target/release/streamflow /usr/local/bin/streamflow
COPY --from=build /opt/target/release/seed-oauth-client /usr/local/bin/seed-oauth-client
COPY --from=build /usr/local/cargo/bin/sqlx /usr/local/bin/sqlx
COPY migrations /opt/migrations

# Copy entrypoint script
COPY docker-entrypoint-deploy.sh /opt/docker-entrypoint-deploy.sh
RUN chmod +x /opt/docker-entrypoint-deploy.sh

EXPOSE 8080
ENTRYPOINT ["/opt/docker-entrypoint-deploy.sh"]
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
