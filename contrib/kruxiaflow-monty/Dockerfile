# kruxiaflow-monty: the add-on worker for Kruxia Flow's sandboxed run_code.
#
# Build from the repository root (the add-on depends on ../../worker-sdk, and
# Cargo reads the engine crates' manifests for its dev-dependencies):
#
#   docker build -f contrib/kruxiaflow-monty/Dockerfile -t kruxia/kruxiaflow-monty .
#
# Run beside the engine, pointed at its API:
#
#   docker run -e KRUXIAFLOW_API_URL=http://kruxiaflow:8080 \
#     -e KRUXIAFLOW_CLIENT_ID=... -e KRUXIAFLOW_CLIENT_SECRET=... kruxia/kruxiaflow-monty

# == BUILD ==
# Rust 1.98+ is required by monty-pool; the engine's own images don't need it
FROM rust:1.100-bookworm AS build
WORKDIR /opt

# The Monty sandbox worker binary, pinned to exactly the version of the
# add-on's monty-pool/monty-types pins: the add-on checks `monty --version`
# at start-up and refuses a mismatch. Stripped at install. Its own layer, so
# it stays cached across source changes.
ARG MONTY_VERSION=1.7.0
RUN CARGO_PROFILE_RELEASE_STRIP=symbols cargo install monty-runtime@${MONTY_VERSION} --locked

COPY ./ ./
WORKDIR /opt/contrib/kruxiaflow-monty
RUN CARGO_PROFILE_RELEASE_STRIP=symbols cargo build --release --locked --bin kruxiaflow-monty

# == DEPLOY ==
# Distroless: the add-on binary and the Monty worker it spawns, nothing else
FROM gcr.io/distroless/cc-debian12:nonroot AS deploy

COPY --from=build /opt/contrib/kruxiaflow-monty/target/release/kruxiaflow-monty /kruxiaflow-monty
COPY --from=build /usr/local/cargo/bin/monty /monty
ENV KRUXIAFLOW_MONTY_BINARY=/monty

ENTRYPOINT ["/kruxiaflow-monty"]
