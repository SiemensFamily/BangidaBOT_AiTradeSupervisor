FROM rust:1.82-slim-bookworm AS builder

WORKDIR /app
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY src/ src/

RUN cargo build --release --bin crypto-scalper

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/crypto-scalper /app/crypto-scalper
COPY config/ /app/config/

ENV RUST_LOG=info
EXPOSE 8080

ENTRYPOINT ["/app/crypto-scalper"]
