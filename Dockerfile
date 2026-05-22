FROM rust:1.90-bookworm AS builder
WORKDIR /src
RUN apt-get update \
    && apt-get install -y --no-install-recommends libudev-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM rust:1.90-bookworm AS test
RUN apt-get update \
    && apt-get install -y --no-install-recommends libudev-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /work

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libudev1 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /src/target/release/reactor-edge-daemon /usr/local/bin/reactor-edge-daemon
COPY config ./config
COPY static ./static
RUN mkdir -p /app/data
EXPOSE 8000
CMD ["/usr/local/bin/reactor-edge-daemon", "--config", "/app/config/device.toml", "--safety", "/app/config/safety.toml", "--memory", "/app/config/ai_memory.toml", "--db", "/app/data/reactor.sqlite3", "--assets", "/app/static", "--bind", "0.0.0.0:8000"]
