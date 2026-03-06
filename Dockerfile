# ============================================================
# StrikeHub — multi-stage Docker build with cargo-chef caching
# ============================================================
# Builds: strikehub-server — liveview web UI + connector hub
#
# Usage:
#   docker build -t strikehub .
#   docker run -p 8080:8080 \
#     -e STRIKE48_API_URL=https://studio.strike48.test \
#     -e STRIKE48_URL=grpc://connectors-studio.strike48.test:80 \
#     -e TENANT_ID=non-prod \
#     -e INSTANCE_ID=strikehub \
#     strikehub

# ----------------------------------------------------------
# Stage 1: cargo-chef planner — compute dependency recipe
# ----------------------------------------------------------
FROM rust:1.91-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ----------------------------------------------------------
# Stage 2: cargo-chef cook — build dependencies (cached layer)
# ----------------------------------------------------------
FROM chef AS builder

# Install build-time system deps (OpenSSL headers for TLS)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

COPY --from=planner /app/recipe.json recipe.json

# Pre-cache deps for the server feature set
RUN cargo chef cook --release --recipe-path recipe.json \
    --features server --no-default-features -p sh-ui

# ----------------------------------------------------------
# Stage 3: Build the actual binary
# ----------------------------------------------------------
COPY . .
RUN cargo build --release --bin strikehub-server --features server --no-default-features -p sh-ui

# ----------------------------------------------------------
# Stage 4: Minimal runtime image
# ----------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 tini \
    && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN groupadd --gid 999 strikehub \
    && useradd --uid 999 --gid strikehub --shell /bin/false -m -d /data/strikehub strikehub

COPY --from=builder /app/target/release/strikehub-server /usr/local/bin/strikehub-server

RUN mkdir -p /tmp && chown strikehub:strikehub /tmp

USER strikehub

ENV PORT=8080
ENV HOME=/data/strikehub
EXPOSE 8080

ENTRYPOINT ["tini", "--"]
CMD ["strikehub-server"]
