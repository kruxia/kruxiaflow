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
RUN cargo install sqlx-cli --no-default-features --features postgres

COPY ./docker-entrypoint-develop.sh /opt/docker-entrypoint-develop.sh
ENTRYPOINT [ "/opt/docker-entrypoint-develop.sh" ]

# Default command - arguments passed to "streamflow serve"
# The entrypoint script will always run: target/profiling/streamflow serve "$@"
EXPOSE 8080

# == DEPLOY ==
FROM develop AS deploy
ARG SQLX_OFFLINE=true
COPY ./ ./
RUN cargo build --release
ENTRYPOINT [ "/opt/docker-entrypoint-deploy.sh" ]
# Default command - arguments passed to "streamflow serve"
EXPOSE 8080

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
RUN cargo install sqlx-cli --no-default-features --features postgres

# Set working directory
WORKDIR /opt

COPY docker-entrypoint-profiling.sh docker-entrypoint-profiling.sh
ENTRYPOINT [ "/opt/docker-entrypoint-profiling.sh" ]

# Default command - arguments passed to "streamflow serve"
# The entrypoint script will always run: target/profiling/streamflow serve "$@"
EXPOSE 8080
