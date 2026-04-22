# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies (OpenSSL headers + pkg-config for reqwest)
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first so dependency layer is cached
COPY Cargo.toml Cargo.lock ./

# Pre-fetch dependencies by building a stub main
RUN mkdir src && echo "fn main(){}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy real source and build
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ── Stage 2: Runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim

WORKDIR /app

# Runtime libraries only
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/aihomeserver /app/aihomeserver

# Data lives on a mounted volume at /data
ENV DATA_DIR=/data
ENV PORT=3000

VOLUME ["/data"]

EXPOSE 3000

CMD ["/app/aihomeserver"]
