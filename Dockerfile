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

# Stage 3: build dependencies, then build iris
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
RUN cargo chef cook --release --recipe-path recipe.json -p iris-cli

# Copy full source and build the actual binary
COPY . .
RUN cargo build --release -p iris-cli && \
    cp target/release/iris /usr/local/bin/iris

# Stage 4: minimal runtime image
FROM ubuntu:24.04 AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3t64 \
    libgomp1 \
    && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN useradd --create-home --home-dir /data iris
USER iris
WORKDIR /data

COPY --from=builder /usr/local/bin/iris /usr/local/bin/iris

ENV RUST_LOG=iris=info
EXPOSE 8080

ENTRYPOINT ["iris", "serve", "--transport", "http", "--host", "0.0.0.0", "--port", "8080"]
