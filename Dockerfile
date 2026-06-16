FROM rust:1-slim-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY *.md ./

RUN mkdir -p src/bin && \
    echo "fn main(){}" > src/main.rs && \
    echo "fn main(){}" > src/bin/worker.rs && \
    cargo build --release --bin aihomeserver --bin worker && \
    rm -rf src

COPY src ./src
RUN cargo build --release --bin aihomeserver --bin worker

FROM debian:bookworm-slim

WORKDIR /app

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/aihomeserver /app/aihomeserver
COPY --from=builder /app/target/release/worker /app/worker
COPY scripts/docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/docker-entrypoint.sh

RUN mkdir -p /repo
COPY *.md Cargo.toml Cargo.lock /repo/
COPY src /repo/src
ENV REPO_ROOT=/repo

ENV DATA_DIR=/data
ENV PORT=3000

VOLUME ["/data"]

EXPOSE 3000 3031

ENTRYPOINT ["/app/docker-entrypoint.sh"]
CMD ["coordinator"]
