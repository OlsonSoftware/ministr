# Stage 1: cargo-chef — compute dependency recipe
# Use Ubuntu 24.04 for GCC 14+ (required by ort-sys ONNX Runtime prebuilts)
FROM rust:1-slim-bookworm AS chef-base
RUN cargo install cargo-chef --locked

FROM ubuntu:24.04 AS chef
COPY --from=chef-base /usr/local/cargo /usr/local/cargo
COPY --from=chef-base /usr/local/rustup /usr/local/rustup
ENV PATH="/usr/local/cargo/bin:${PATH}" \
    RUSTUP_HOME="/usr/local/rustup" \
    CARGO_HOME="/usr/local/cargo"
WORKDIR /app

# Stage 2: plan dependencies (cache-friendly layer)
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: build dependencies, then build ministr
FROM chef AS builder

# Install build-time system dependencies (Ubuntu 24.04 ships GCC 14)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    pkg-config \
    libssl-dev \
    cmake \
    g++ \
    && rm -rf /var/lib/apt/lists/*

# Cook dependencies from recipe (cached unless Cargo.toml/lock change)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p ministr-cli

# Copy full source and build the actual binary
COPY . .
RUN cargo build --release -p ministr-cli && \
    cp target/release/ministr /usr/local/bin/ministr

# Stage 4: minimal runtime image
FROM ubuntu:24.04 AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3t64 \
    libgomp1 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN useradd --create-home --home-dir /data ministr
WORKDIR /data

COPY --from=builder /usr/local/bin/ministr /usr/local/bin/ministr
COPY deploy/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

USER ministr

ENV RUST_LOG=ministr=info
EXPOSE 8080

# Honoured by `docker run` (ACA uses its own httpGet probes from the
# Pulumi template). Same `/healthz` endpoint either way.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://localhost:8080/healthz || exit 1

# ENTRYPOINT_MODE selects between `serve` (default) and `index`. See
# deploy/docker-entrypoint.sh.
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
